use crate::api::ble_service::use_ble;
use crate::ble_constants::{
    BATTERY_LEVEL_UUID, BATTERY_SERVICE_UUID, PS_DATA_UUID, PS_FREQ_MAX, PS_FREQ_UUID, PS_INT_UUID,
    RELOAD_INT_UUID, SERVICE_UUID,
};
use crate::context::use_app_state;
use dioxus::{core::Task, prelude::*};
use futures::StreamExt;
use std::time::Duration;
use tokio::time::{timeout_at, Instant};

const DEFAULT_POLL_HZ: i32 = 5;
const READ_RETRY_DELAY: Duration = Duration::from_secs(1);
const SUBSCRIBE_RETRY_DELAY: Duration = Duration::from_secs(3);
const FREQUENCY_WRITE_DELAY: Duration = Duration::from_millis(500);

#[component]
pub fn Status() -> Element {
    let app_state = use_app_state();
    let device_id = app_state.connected_device_id.read().clone();
    let theme = (app_state.theme)();

    rsx! {
        div {
            class: theme.pick(
                "app-scroll h-full w-full text-gray-100 overflow-auto relative",
                "app-scroll h-full w-full text-gray-900 overflow-auto relative",
            ),
            if device_id.is_empty() {
                StatusEmptyState {}
            } else {
                ConnectedStatus { key: "{device_id}", device_id }
            }
        }
    }
}

#[component]
fn ConnectedStatus(device_id: String) -> Element {
    let ble = use_ble();
    let mut battery_level = use_signal(|| 0);
    let mut ps_samples = use_signal(Vec::<u16>::new);
    let mut ps_error = use_signal(|| None::<String>);
    let mut ps_frequency = use_signal(|| None::<i32>);
    let mut frequency_error = use_signal(|| None::<String>);
    let mut frequency_input = use_signal(|| None::<String>);

    let id = device_id.clone();
    use_future(move || {
        let id = id.clone();
        async move {
            match ble
                .read(&id, BATTERY_SERVICE_UUID, BATTERY_LEVEL_UUID)
                .await
            {
                Ok(data) => {
                    if let Some(level) = parse_battery_level(&data) {
                        battery_level.set(level as i32);
                    }
                }
                Err(e) => error!("Failed to read battery: {e}"),
            }
            match ble
                .subscribe(&id, BATTERY_SERVICE_UUID, BATTERY_LEVEL_UUID)
                .await
            {
                Ok(mut stream) => {
                    while let Some(data) = stream.next().await {
                        if let Some(level) = parse_battery_level(&data.value) {
                            battery_level.set(level as i32);
                        }
                    }
                }
                Err(e) => error!("Failed to subscribe to battery: {e}"),
            }
        }
    });

    let id = device_id.clone();
    let write_frequency =
        use_coroutine(move |mut rx: UnboundedReceiver<(Option<i32>, Instant)>| {
            let id = id.clone();
            async move {
                let result = ble
                    .read(&id, SERVICE_UUID, PS_FREQ_UUID)
                    .await
                    .and_then(|data| {
                        parse_ps_frequency(&data).ok_or_else(|| {
                            format!("0xFFE9 应返回 1 字节，实际返回 {} 字节", data.len())
                        })
                    });
                match result {
                    Ok(frequency) => ps_frequency.set(Some(frequency)),
                    Err(error) => frequency_error.set(Some(error)),
                }

                while let Some(value) = next_frequency_write(&mut rx).await {
                    if *ps_frequency.peek() == Some(value) {
                        frequency_error.set(None);
                        continue;
                    }
                    match ble
                        .write(&id, SERVICE_UUID, PS_FREQ_UUID, vec![value as u8])
                        .await
                    {
                        Ok(()) => {
                            ps_frequency.set(Some(value));
                            frequency_error.set(None);
                        }
                        Err(error) => frequency_error.set(Some(error)),
                    }
                }
            }
        });

    let id = device_id.clone();
    use_future(move || {
        let id = id.clone();
        async move {
            let mut first_read = true;
            loop {
                let poll_hz = ps_frequency().unwrap_or(DEFAULT_POLL_HZ);
                // Zero disables periodic reads. Still show one snapshot when opening the page.
                if poll_hz == 0 && !first_read {
                    tokio::time::sleep(READ_RETRY_DELAY).await;
                    continue;
                }
                first_read = false;
                let result = ble
                    .read(&id, SERVICE_UUID, PS_DATA_UUID)
                    .await
                    .and_then(|data| {
                        parse_ps_value(&data).ok_or_else(|| {
                            format!("0xFFE7 应返回 2 字节，实际返回 {} 字节", data.len())
                        })
                    });
                let delay = match result {
                    Ok(value) => {
                        if ps_samples.peek().is_empty() {
                            info!("Received first PS reading: {value}");
                        }
                        ps_samples.with_mut(|readings| {
                            if readings.len() >= 50 {
                                readings.remove(0);
                            }
                            readings.push(value);
                        });
                        if ps_error.peek().is_some() {
                            ps_error.set(None);
                        }
                        Duration::from_millis(1000 / poll_hz.max(1) as u64)
                    }
                    Err(error) => {
                        if ps_error.peek().as_deref() != Some(error.as_str()) {
                            error!("Failed to read PS value: {error}");
                            ps_error.set(Some(error));
                        }
                        READ_RETRY_DELAY
                    }
                };
                tokio::time::sleep(delay).await;
            }
        }
    });

    let readings = ps_samples();
    let frequency = ps_frequency();
    let last_reading = readings.last().copied();
    let frequency_error = frequency_error();
    let frequency_loading = frequency.is_none() && frequency_error.is_none();
    let freq_value = frequency_input()
        .or_else(|| frequency.map(|value| value.to_string()))
        .unwrap_or_default();

    rsx! {
        div { class: "w-full max-w-5xl mx-auto space-y-5",
            StatusHeader { battery_level: battery_level() }
            SensorCard {
                current_value: last_reading,
                data: readings,
                read_error: ps_error(),
                paused: frequency == Some(0),
                freq_value,
                frequency_error,
                frequency_loading,
                on_freq_input: move |input: String| {
                    let value = parse_frequency_input(&input);
                    frequency_input.set(Some(input));
                    write_frequency.send((value, Instant::now() + FREQUENCY_WRITE_DELAY));
                },
            }
            TriggerCounters { device_id }
        }
    }
}

fn parse_ps_value(data: &[u8]) -> Option<u16> {
    match data {
        [low, high] => Some(u16::from_le_bytes([*low, *high])),
        _ => None,
    }
}

fn parse_ps_frequency(data: &[u8]) -> Option<i32> {
    match data {
        [frequency] => Some((frequency & 0x0f).min(PS_FREQ_MAX as u8) as i32),
        _ => None,
    }
}

fn parse_frequency_input(input: &str) -> Option<i32> {
    input
        .parse()
        .ok()
        .filter(|value| (0..=PS_FREQ_MAX).contains(value))
}

async fn next_frequency_write(
    edits: &mut UnboundedReceiver<(Option<i32>, Instant)>,
) -> Option<i32> {
    while let Some((mut value, mut deadline)) = edits.next().await {
        // Clearing the field or entering an invalid value also cancels the pending write.
        while let Ok(next) = timeout_at(deadline, edits.next()).await {
            (value, deadline) = next?;
        }
        if value.is_some() {
            return value;
        }
    }
    None
}

fn parse_battery_level(data: &[u8]) -> Option<u8> {
    match data {
        [level] => Some((*level).min(100)),
        _ => None,
    }
}

fn parse_interrupt_notification(data: &[u8]) -> Option<bool> {
    match data {
        [flag] => Some(*flag != 0),
        _ => None,
    }
}

#[component]
fn StatusEmptyState() -> Element {
    let theme = *use_app_state().theme.read();
    rsx! {
        div { class: "flex items-center justify-center h-full min-h-[360px]",
            div { class: theme.pick("empty-panel max-w-sm rounded-2xl border border-white/10 bg-white/[0.03] px-8 py-10 text-center shadow-xl", "empty-panel max-w-sm rounded-2xl border border-slate-200 bg-white px-8 py-10 text-center shadow-sm"),
                div { class: "mx-auto flex h-14 w-14 items-center justify-center rounded-2xl bg-[#1d3040] text-2xl text-[#b8f36b]", "◌" }
                div { class: theme.pick("mt-5 text-lg font-semibold text-slate-200", "mt-5 text-lg font-semibold text-slate-800"), "未连接设备" }
                div { class: theme.pick("mt-2 text-sm text-slate-400", "mt-2 text-sm text-slate-500"), "请先前往连接页面查看设备状态" }
            }
        }
    }
}

#[component]
fn StatusHeader(battery_level: i32) -> Element {
    let (battery_color, battery_bg) = get_battery_style(battery_level);

    rsx! {
        div { class: "page-header flex items-end justify-between gap-4",
            h1 { class: "text-2xl font-semibold tracking-tight", "设备状态" }
            BatteryIndicator { level: battery_level, color: battery_color, bg: battery_bg }
        }
    }
}

#[component]
fn BatteryIndicator(level: i32, color: &'static str, bg: &'static str) -> Element {
    let app_state = use_app_state();
    let theme = (app_state.theme)();
    let level = level.clamp(0, 100);
    rsx! {
        div {
            class: theme
                .pick(
                    "flex items-center gap-2 px-3 py-2 rounded-xl border border-white/10 bg-white/[0.04]",
                    "flex items-center gap-2 px-3 py-2 rounded-xl border border-slate-200 bg-white shadow-sm",
                ),
            div { class: "flex items-center gap-2",
                div { class: "relative w-8 h-4 border border-gray-400 rounded",
                    div {
                        class: "absolute top-0 left-0 h-full rounded-sm transition-all duration-300 {bg}",
                        style: "width: {level}%",
                    }
                    div { class: "absolute -right-0.5 top-1/2 -translate-y-1/2 w-0.5 h-1.5 bg-gray-400 rounded-r" }
                }
                span { class: "text-sm font-semibold {color}", "{level}%" }
            }
        }
    }
}

#[component]
fn SensorCard(
    current_value: Option<u16>,
    data: Vec<u16>,
    read_error: Option<String>,
    paused: bool,
    freq_value: String,
    frequency_error: Option<String>,
    frequency_loading: bool,
    on_freq_input: EventHandler<String>,
) -> Element {
    let theme = (use_app_state().theme)();
    let frequency_invalid = !freq_value.is_empty() && parse_frequency_input(&freq_value).is_none();

    rsx! {
        div {
            class: theme.pick(
                "panel rounded-2xl border border-white/10 bg-white/[0.04] p-5 space-y-3 shadow-xl shadow-black/10",
                "panel rounded-2xl border border-slate-200 bg-white p-5 space-y-3 shadow-sm",
            ),
            div {
                div { class: "flex items-center justify-between mb-1",
                    h2 { class: "text-base font-medium", "PS 传感器数据" }
                    div { class: theme.pick(
                        "flex items-center gap-2 text-xs text-slate-400",
                        "flex items-center gap-2 text-xs text-slate-500",
                    ),
                        label { r#for: "ps-frequency", "刷新" }
                        input {
                            id: "ps-frequency",
                            class: theme.pick("sensor-frequency-input sensor-frequency-input-dark", "sensor-frequency-input sensor-frequency-input-light"),
                            r#type: "number",
                            min: "0",
                            max: "{PS_FREQ_MAX}",
                            value: "{freq_value}",
                            disabled: frequency_loading,
                            aria_invalid: frequency_invalid,
                            step: "1",
                            inputmode: "numeric",
                            oninput: move |e| on_freq_input.call(e.value()),
                        }
                        span { "次/秒" }
                        span { class: "text-[10px] text-slate-500", "0=停止" }
                    }
                }
                p { class: "text-[10px] text-gray-500", "接近传感器实时数据 (UUID: 0xFFE7)" }
                if frequency_invalid {
                    p { class: theme.pick("text-xs text-red-300", "text-xs text-red-700"),
                        "请输入 0–{PS_FREQ_MAX} 的整数"
                    }
                }
                if let Some(error) = &frequency_error {
                    p {
                        class: theme.pick("text-xs text-red-300", "text-xs text-red-700"),
                        style: "overflow-wrap: anywhere",
                        "刷新频率操作失败 · {error}"
                    }
                }
            }
            div { class: theme.pick("sensor-reading sensor-reading-dark", "sensor-reading sensor-reading-light"),
                span { class: "sensor-reading-label",
                    if current_value.is_some() && (paused || read_error.is_some()) {
                        "最后读数"
                    } else {
                        "当前值"
                    }
                }
                div { class: "sensor-reading-value-group",
                    if let Some(value) = current_value {
                        div { class: "sensor-reading-value tabular-nums text-[#60cd18]", "{value}" }
                    } else if read_error.is_some() {
                        div { class: "sensor-reading-empty", "读取失败" }
                    } else {
                        div { class: "sensor-reading-empty", "正在读取…" }
                    }
                    if paused {
                        div { class: "sensor-reading-scale", "周期读取已暂停" }
                    }
                }
            }
            if let Some(error) = read_error {
                p {
                    class: theme.pick("text-xs text-red-300", "text-xs text-red-700"),
                    style: "overflow-wrap: anywhere",
                    "PS 读取失败 · {error}"
                }
            }
            ChartView { data }
        }
    }
}

#[component]
fn TriggerCounters(device_id: String) -> Element {
    let theme = (use_app_state().theme)();
    rsx! {
        div {
            class: theme.pick(
                "panel rounded-2xl border border-white/10 bg-white/[0.04] p-5",
                "panel rounded-2xl border border-slate-200 bg-white p-5 shadow-sm",
            ),
            h2 { class: "mb-3 text-base font-medium", "中断通知计数器" }
            div { class: "grid grid-cols-1 md:grid-cols-2 gap-4",
                TriggerCounter {
                    device_id: device_id.clone(),
                    uuid: PS_INT_UUID,
                    label: "PS 中断通知",
                    color: "emerald",
                }
                TriggerCounter {
                    device_id,
                    uuid: RELOAD_INT_UUID,
                    label: "Reload 中断通知",
                    color: "orange",
                }
            }
        }
    }
}

#[component]
fn TriggerCounter(
    device_id: String,
    uuid: &'static str,
    label: &'static str,
    color: &'static str,
) -> Element {
    let ble = use_ble();
    let mut count = use_signal(|| None::<u32>);
    let mut active = use_signal(|| false);
    let mut notification_status = use_signal(|| None::<Result<(), String>>);
    use_future(move || {
        let id = device_id.clone();
        async move {
            let mut flash: Option<Task> = None;
            loop {
                let error = match ble.subscribe(&id, SERVICE_UUID, uuid).await {
                    Ok(mut stream) => {
                        notification_status.set(Some(Ok(())));
                        if count.peek().is_none() {
                            count.set(Some(0));
                        }
                        while let Some(data) = stream.next().await {
                            let Some(triggered) = parse_interrupt_notification(&data.value) else {
                                notification_status.set(Some(Err(format!(
                                    "通知应包含 1 字节，实际收到 {} 字节",
                                    data.value.len()
                                ))));
                                continue;
                            };
                            if matches!(&*notification_status.peek(), Some(Err(_))) {
                                notification_status.set(Some(Ok(())));
                            }
                            if triggered {
                                count.with_mut(|value| {
                                    *value = Some(value.unwrap_or_default().saturating_add(1));
                                });
                                if let Some(task) = flash.take() {
                                    task.cancel();
                                }
                                active.set(true);
                                flash = Some(spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(150)).await;
                                    active.set(false);
                                }));
                            }
                        }
                        "通知流已关闭".to_string()
                    }
                    Err(error) => error,
                };
                if !matches!(&*notification_status.peek(), Some(Err(previous)) if previous == &error)
                {
                    error!("Failed to receive notifications for {uuid}: {error}");
                    notification_status.set(Some(Err(error)));
                }
                tokio::time::sleep(SUBSCRIBE_RETRY_DELAY).await;
            }
        }
    });

    let (text_color, border_color, bg_color, ripple_color) = get_trigger_colors(color);
    let app_state = use_app_state();
    let theme = (app_state.theme)();
    let is_active = active();
    let count_label = count().map_or_else(|| "—".to_string(), |value| value.to_string());
    let notice = notification_status();
    let subtitle = match &notice {
        Some(Ok(())) => "正在监听",
        Some(Err(_)) => "通知接收失败",
        None => "正在订阅…",
    };
    let inactive_card = theme.pick(
        "rounded-xl border border-white/10 bg-[#101d2b] p-4 space-y-1 transition-all duration-150",
        "rounded-xl border border-slate-200 bg-slate-50 p-4 space-y-1 transition-all duration-150",
    );

    let card_class = if is_active {
        format!(
            "rounded-xl border-2 {} {} p-4 space-y-1 transition-all duration-150",
            border_color, bg_color
        )
    } else {
        inactive_card.to_string()
    };

    rsx! {
        div { class: "relative",
            // 波纹效果 - 使用 pointer-events-none 避免影响布局
            if is_active {
                div {
                    class: "absolute inset-0 rounded-lg animate-ripple pointer-events-none z-0",
                    style: "--ripple-color: {ripple_color};",
                }
                div {
                    class: "absolute inset-0 rounded-lg pointer-events-none z-0",
                    style: "box-shadow: 0 0 30px 10px {ripple_color}; opacity: 0.6;",
                }
            }

            // 卡片内容 - 固定高度避免布局变化
            div { class: "{card_class} relative h-[128px] flex flex-col justify-between z-10",
                // 标题区域 - 固定高度
                div { class: "flex flex-col gap-0.5 min-h-8",
                    div { class: "flex items-center justify-between min-h-4",
                        div { class: "text-xs text-gray-400 leading-none", "{label}" }
                        div { class: "flex items-center justify-end min-w-16 min-h-4",
                            div {
                                class: format!(
                                    "text-[10px] px-1.5 py-0.5 rounded-full {} {} font-semibold leading-none transition-opacity duration-150 {}",
                                    bg_color,
                                    text_color,
                                    if is_active { "opacity-100 animate-pulse" } else { "opacity-0" },
                                ),
                                "触发!"
                            }
                        }
                    }
                    div { class: "text-[10px] text-gray-500 leading-none min-h-3", "{subtitle}" }
                }

                // 固定高度的数字容器 - 不再缩放字号，只改颜色阴影
                div { class: "h-12 flex items-center overflow-hidden",
                    div {
                        class: format!(
                            "text-2xl font-bold {} transition-all duration-150 leading-none",
                            text_color,
                        ),
                        style: if is_active { "text-shadow: 0 0 12px rgba(255,255,255,0.35);" } else { "" },
                        "{count_label}"
                    }
                }

                // 底部操作栏 - 固定高度
                div { class: "h-6 flex items-center",
                    button {
                        class: theme
                            .pick(
                                "text-[10px] px-2 py-0.5 rounded bg-[#2a2a2a] hover:bg-[#333] cursor-pointer text-gray-300 transition-colors",
                                "text-[10px] px-2 py-0.5 rounded bg-gray-100 hover:bg-gray-200 cursor-pointer text-gray-600 transition-colors",
                            ),
                        disabled: count().is_none(),
                        onclick: move |_| count.set(Some(0)),
                        "重置"
                    }
                }
            }
            if let Some(Err(error)) = notice {
                p {
                    class: theme.pick("mt-2 text-xs text-red-300", "mt-2 text-xs text-red-700"),
                    style: "overflow-wrap: anywhere",
                    "{error}"
                }
            }
        }
    }
}

fn get_trigger_colors(color: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    match color {
        "emerald" => (
            "text-emerald-400",
            "border-emerald-500",
            "bg-emerald-500/20",
            "rgba(16, 185, 129, 0.6)", // emerald-500
        ),
        "blue" => (
            "text-blue-400",
            "border-blue-500",
            "bg-blue-500/20",
            "rgba(59, 130, 246, 0.6)", // blue-500
        ),
        "purple" => (
            "text-purple-400",
            "border-purple-500",
            "bg-purple-500/20",
            "rgba(168, 85, 247, 0.6)", // purple-500
        ),
        "orange" => (
            "text-orange-400",
            "border-orange-500",
            "bg-orange-500/20",
            "rgba(249, 115, 22, 0.6)", // orange-500
        ),
        _ => (
            "text-gray-400",
            "border-gray-500",
            "bg-gray-500/20",
            "rgba(107, 114, 128, 0.6)", // gray-500
        ),
    }
}

#[component]
fn ChartView(data: Vec<u16>) -> Element {
    let scale = ChartScale::from_samples(&data);
    let (line_path, fill_path) = generate_chart_paths(&data, scale);
    let theme = (use_app_state().theme)();
    let line_color = theme.pick("#60cd18", "#3b8314");
    let grid_color = theme.pick("#33475a", "#d3dee7");

    rsx! {
        div {
            class: theme.pick(
                "ps-chart h-40 bg-[#0b1521] rounded-xl p-3 border border-white/10 text-slate-400",
                "ps-chart h-40 bg-slate-50 rounded-xl p-3 border border-slate-200 text-slate-600",
            ),
            div { class: "ps-chart-axis",
                for tick in scale.ticks() {
                    span { key: "{tick}", "{tick}" }
                }
            }
            div { class: "ps-chart-plot",
                svg {
                    view_box: "0 0 {CHART_WIDTH} {CHART_HEIGHT}",
                    preserve_aspect_ratio: "none",
                    role: "img",
                    "aria-label": "最近的 PS 读数曲线",
                    for tick in scale.ticks() {
                        line {
                            key: "{tick}",
                            x1: "0",
                            y1: "{scale.y(tick)}",
                            x2: "{CHART_WIDTH}",
                            y2: "{scale.y(tick)}",
                            stroke: "{grid_color}",
                            stroke_width: "1",
                            vector_effect: "non-scaling-stroke",
                        }
                    }
                    path { d: "{fill_path}", fill: "{line_color}", fill_opacity: "0.12" }
                    path {
                        d: "{line_path}",
                        fill: "none",
                        stroke: "{line_color}",
                        stroke_width: "2",
                        stroke_linejoin: "round",
                        stroke_linecap: "round",
                        vector_effect: "non-scaling-stroke",
                    }
                }
            }
        }
    }
}

const CHART_WIDTH: f64 = 100.0;
const CHART_HEIGHT: f64 = 150.0;

#[derive(Clone, Copy)]
struct ChartScale {
    min: u32,
    max: u32,
    step: u32,
}

impl ChartScale {
    fn from_samples(samples: &[u16]) -> Self {
        let min = u32::from(samples.iter().copied().min().unwrap_or_default());
        let max = u32::from(samples.iter().copied().max().unwrap_or_default());
        // Leave a margin around the samples, even when every reading is equal.
        let padding = (max - min).div_ceil(10).max(2);
        let lower = min.saturating_sub(padding);
        let upper = (max + padding).max(lower + 4);
        let approximate_step = (upper - lower).div_ceil(4);
        let magnitude = 10_u32.pow(approximate_step.ilog10());
        let step = match approximate_step.div_ceil(magnitude) {
            1 => magnitude,
            2 => 2 * magnitude,
            3..=5 => 5 * magnitude,
            _ => 10 * magnitude,
        };
        Self {
            min: lower / step * step,
            max: upper.next_multiple_of(step),
            step,
        }
    }

    fn ticks(self) -> impl Iterator<Item = u32> {
        (self.min..=self.max).rev().step_by(self.step as usize)
    }

    fn y(self, value: u32) -> f64 {
        CHART_HEIGHT * f64::from(self.max - value) / f64::from(self.max - self.min)
    }
}

fn generate_chart_paths(data: &[u16], scale: ChartScale) -> (String, String) {
    use std::fmt::Write as _;

    let first = match data {
        [] => return (String::new(), String::new()),
        [value] => {
            let x = CHART_WIDTH / 2.0;
            let y = scale.y(u32::from(*value));
            // A round, zero-length stroke marks the first sample until a line can be drawn.
            return (format!("M {x:.2} {y:.2} L {x:.2} {y:.2}"), String::new());
        }
        [first, ..] => *first,
    };

    let mut line_path = format!("M 0 {:.2}", scale.y(u32::from(first)));
    for (i, value) in data.iter().enumerate().skip(1) {
        let x = CHART_WIDTH * i as f64 / (data.len() - 1) as f64;
        let y = scale.y(u32::from(*value));
        let _ = write!(line_path, " L {x:.2} {y:.2}");
    }

    let fill_path = format!("{line_path} L {CHART_WIDTH} {CHART_HEIGHT} L 0 {CHART_HEIGHT} Z");
    (line_path, fill_path)
}

fn get_battery_style(level: i32) -> (&'static str, &'static str) {
    if level > 50 {
        ("text-emerald-400", "bg-emerald-500")
    } else if level > 20 {
        ("text-yellow-400", "bg-yellow-500")
    } else {
        ("text-red-400", "bg-red-500")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        generate_chart_paths, next_frequency_write, parse_battery_level, parse_frequency_input,
        parse_interrupt_notification, parse_ps_frequency, parse_ps_value, ChartScale, CHART_HEIGHT,
        FREQUENCY_WRITE_DELAY,
    };
    use tokio::time::Instant;

    #[test]
    fn ps_reading_requires_exactly_two_little_endian_bytes() {
        assert_eq!(parse_ps_value(&[0x34, 0x12]), Some(0x1234));
        assert_eq!(parse_ps_value(&[0x2a]), None);
        assert_eq!(parse_ps_value(&[]), None);
        assert_eq!(parse_ps_value(&[0x34, 0x12, 0]), None);
    }

    #[test]
    fn chart_scale_fits_recent_samples_and_uses_the_same_range_for_ticks_and_points() {
        let low = ChartScale::from_samples(&[0, 1, 3, 1]);
        assert_eq!(low.ticks().collect::<Vec<_>>(), [6, 4, 2, 0]);
        assert_eq!(low.y(3), CHART_HEIGHT / 2.0);
        assert_eq!(low.y(0), CHART_HEIGHT);

        let high = ChartScale::from_samples(&[0x1234, 0x1235]);
        assert!(high.min > 0x1200);
        assert!(high.y(0x1234) - high.y(0x1235) >= CHART_HEIGHT / 6.0);
        assert_eq!(high.ticks().last(), Some(high.min));

        let full = ChartScale::from_samples(&[0, u16::MAX]);
        assert!(full.y(u32::from(u16::MAX)) < CHART_HEIGHT / 2.0);
        assert_eq!(full.min, 0);
    }

    #[test]
    fn chart_handles_empty_and_constant_samples() {
        let scale = ChartScale::from_samples(&[]);
        assert!(scale.max > scale.min);
        assert!(generate_chart_paths(&[], scale).0.is_empty());
        let zero = ChartScale::from_samples(&[0]);
        let (line, fill) = generate_chart_paths(&[0], zero);
        assert_eq!(line, "M 50.00 150.00 L 50.00 150.00");
        assert!(fill.is_empty());

        let flat = ChartScale::from_samples(&[u16::MAX, u16::MAX]);
        assert_eq!(flat.y(u32::from(u16::MAX)), CHART_HEIGHT / 2.0);
    }

    #[test]
    fn ps_frequency_uses_the_low_nibble_and_zero_means_stopped() {
        assert_eq!(parse_ps_frequency(&[0]), Some(0));
        assert_eq!(parse_ps_frequency(&[15]), Some(15));
        assert_eq!(parse_ps_frequency(&[0xf5]), Some(5));
        assert_eq!(parse_ps_frequency(&[]), None);
        assert_eq!(parse_ps_frequency(&[1, 2]), None);
    }

    #[tokio::test]
    async fn frequency_edits_apply_only_the_latest_valid_value_after_a_pause() {
        let (tx, mut edits) = futures::channel::mpsc::unbounded();
        tx.unbounded_send((
            parse_frequency_input("1"),
            Instant::now() + FREQUENCY_WRITE_DELAY,
        ))
        .unwrap();
        let write = next_frequency_write(&mut edits);
        tokio::pin!(write);
        assert!(futures::poll!(&mut write).is_pending());

        tx.unbounded_send((parse_frequency_input("15"), Instant::now()))
            .unwrap();
        assert_eq!(write.await, Some(15));
    }

    #[tokio::test]
    async fn clearing_or_exceeding_the_frequency_range_cancels_a_pending_write() {
        let (tx, mut edits) = futures::channel::mpsc::unbounded();
        tx.unbounded_send((parse_frequency_input("1"), Instant::now()))
            .unwrap();
        tx.unbounded_send((parse_frequency_input("16"), Instant::now()))
            .unwrap();
        tx.unbounded_send((parse_frequency_input(""), Instant::now()))
            .unwrap();

        let write = next_frequency_write(&mut edits);
        tokio::pin!(write);
        assert!(futures::poll!(&mut write).is_pending());
        tx.unbounded_send((parse_frequency_input("0"), Instant::now()))
            .unwrap();
        assert_eq!(write.await, Some(0));
    }

    #[test]
    fn battery_and_interrupt_values_require_one_byte() {
        assert_eq!(parse_battery_level(&[85]), Some(85));
        assert_eq!(parse_battery_level(&[150]), Some(100));
        assert_eq!(parse_battery_level(&[85, 0]), None);
        assert_eq!(parse_interrupt_notification(&[1]), Some(true));
        assert_eq!(parse_interrupt_notification(&[0]), Some(false));
        assert_eq!(parse_interrupt_notification(&[]), None);
        assert_eq!(parse_interrupt_notification(&[1, 0]), None);
    }
}
