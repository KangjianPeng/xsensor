use crate::api::ble_service::use_ble;
use crate::ble_constants::{
    BATTERY_LEVEL_UUID, BATTERY_SERVICE_UUID, PS_DATA_UUID, PS_FREQ_MAX, PS_FREQ_UUID, PS_INT_UUID,
    RELOAD_INT_UUID, SERVICE_UUID,
};
use crate::context::use_app_state;
use dioxus::prelude::*;
use futures::StreamExt;

// FFE7 is a two-byte value. Keep the complete u16 range so values whose high
// byte is non-zero are displayed instead of being folded back to zero.
const PS_CHART_BASE_MAX: u16 = 0x0fff;

#[component]
pub fn Status() -> Element {
    let app_state = use_app_state();
    let ble = use_ble();
    let device_id = app_state.connected_device_id.read().clone();

    let mut battery_level = use_signal(|| 0);
    // Start with no samples. A synthetic zero history makes an unavailable
    // sensor look like it is returning a real value of zero.
    let mut sensor_data_1 = use_signal(Vec::<u16>::new); // PS Data (FFE7, two-byte value)
    let mut trigger_count_1 = use_signal(|| 0); // PS Interrupt
    let mut trigger_active_1 = use_signal(|| false);
    let mut trigger_count_2 = use_signal(|| 0); // Reload Interrupt
    let mut trigger_active_2 = use_signal(|| false);
    let mut ps_refresh_freq = use_signal(|| 5); // 刷新频率
    let mut is_loaded = use_signal(|| false);

    // 取消标志：组件卸载时设为 true，让所有后台订阅任务退出
    // use_resource 创建的任务并不随组件卸载而自动取消，
    // 使用 use_hook 存储 DropGuard，在 scope drop 时触发清理
    let cancelled = use_signal(|| false);
    use_hook(|| {
        // Clone 返回 active=false 副本，这样 use_hook 返回的克隆被丢弃时不会迎早触发取消。
        // 只有 use_hook 中存储的原始实例（active=true）在 scope 释放时才进行清理。
        struct DropGuard {
            sig: Signal<bool>,
            active: bool,
        }
        impl Clone for DropGuard {
            fn clone(&self) -> Self {
                DropGuard {
                    sig: self.sig,
                    active: false,
                }
            }
        }
        impl Drop for DropGuard {
            fn drop(&mut self) {
                if self.active {
                    self.sig.set(true);
                }
            }
        }
        DropGuard {
            sig: cancelled,
            active: true,
        }
    });

    let connected_device_id = app_state.connected_device_id;
    let write_ps_freq_task = use_coroutine(move |mut rx: UnboundedReceiver<i32>| async move {
        while let Some(new_val) = rx.next().await {
            let id = connected_device_id.read().clone();
            if id.is_empty() {
                error!("Cannot write PS frequency without a connected device");
                continue;
            }
            let freq_byte = new_val.clamp(0, PS_FREQ_MAX) as u8;
            if let Err(e) = ble
                .write(&id, SERVICE_UUID, PS_FREQ_UUID, vec![freq_byte])
                .await
            {
                error!("Failed to write PS freq: {}", e);
            }
        }
    });
    let on_ps_freq_change = move |new_val: i32| {
        ps_refresh_freq.set(new_val);
        write_ps_freq_task.send(new_val);
    };

    // 读取刷新频率和电池电量
    use_resource(move || {
        let id = app_state.connected_device_id.read().clone();
        async move {
            if id.is_empty() {
                battery_level.set(0);
                sensor_data_1.set(Vec::new());
                trigger_count_1.set(0);
                trigger_count_2.set(0);
                trigger_active_1.set(false);
                trigger_active_2.set(false);
                is_loaded.set(false);
                return;
            }

            battery_level.set(0);
            sensor_data_1.set(Vec::new());
            trigger_count_1.set(0);
            trigger_count_2.set(0);
            trigger_active_1.set(false);
            trigger_active_2.set(false);
            ps_refresh_freq.set(5);
            is_loaded.set(false);

            // Wait for services to be discovered
            let mut services_ready = false;
            for _ in 0..20 {
                if let Ok(services) = ble.list_characteristics(id.clone()).await {
                    if services.iter().any(|s| s.uuid == SERVICE_UUID) {
                        services_ready = true;
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            if !services_ready {
                is_loaded.set(true);
                return;
            }

            // 读取刷新频率特性 (0xFFE9)
            match ble.read(&id, SERVICE_UUID, PS_FREQ_UUID).await {
                Ok(data) => {
                    if let Some(frequency) = parse_ps_frequency(&data) {
                        // Zero intentionally disables periodic reads and must
                        // not fall back to the default.
                        ps_refresh_freq.set(frequency);
                    } else {
                        error!(
                            "PS frequency characteristic returned an invalid value: {data:02X?}"
                        );
                    }
                }
                Err(e) => {
                    error!("Failed to read freq: {}", e);
                }
            }

            // 读取电池电量 (0x2A19)
            match ble
                .read(&id, BATTERY_SERVICE_UUID, BATTERY_LEVEL_UUID)
                .await
            {
                Ok(data) => {
                    if let Some(level) = parse_battery_level(&data) {
                        battery_level.set(level as i32);
                    }
                }
                Err(e) => {
                    error!("Failed to read battery: {}", e);
                }
            }

            is_loaded.set(true);
        }
    });

    // Battery Subscription
    use_resource(move || {
        let id = app_state.connected_device_id.read().clone();
        async move {
            if id.is_empty() {
                return;
            }
            // Wait for services to be discovered
            let mut battery_service_ready = false;
            for _ in 0..20 {
                if let Ok(services) = ble.list_characteristics(id.clone()).await {
                    if services.iter().any(|s| s.uuid == BATTERY_SERVICE_UUID) {
                        battery_service_ready = true;
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            if !battery_service_ready {
                return;
            }
            match ble
                .subscribe(&id, BATTERY_SERVICE_UUID, BATTERY_LEVEL_UUID)
                .await
            {
                Ok(mut stream) => {
                    while let Some(data) = stream.next().await {
                        if cancelled() {
                            break;
                        }
                        if let Some(level) = parse_battery_level(&data.value) {
                            battery_level.set(level as i32);
                        }
                    }
                }
                Err(e) => error!("Failed to subscribe battery: {}", e),
            }
        }
    });

    // PS Polling
    use_resource(move || {
        let id = app_state.connected_device_id.read().clone();
        async move {
            if id.is_empty() {
                return;
            }
            // Wait for the PS data characteristic to be discovered.
            loop {
                if cancelled() {
                    return;
                }
                if let Ok(services) = ble.list_characteristics(id.clone()).await {
                    let available = services
                        .iter()
                        .find(|service| service.uuid == SERVICE_UUID)
                        .and_then(|service| {
                            service
                                .characteristic
                                .iter()
                                .find(|characteristic| characteristic.uuid == PS_DATA_UUID)
                        })
                        .is_some();
                    if available {
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            let mut first_sample = true;
            let mut invalid_sample_logged = false;
            loop {
                if cancelled() {
                    break;
                }
                let freq = ps_refresh_freq();
                if freq > 0 {
                    // FFE7 is defined by the device protocol as a two-byte read
                    // characteristic. Do not select notification mode based on
                    // incomplete or inconsistent metadata from Windows BLE.
                    match ble.read(&id, SERVICE_UUID, PS_DATA_UUID).await {
                        Ok(data) => {
                            update_ps_data(
                                &data,
                                &mut sensor_data_1,
                                &mut first_sample,
                                &mut invalid_sample_logged,
                                "read",
                            );
                        }
                        Err(e) => error!("Failed to read PS data: {}", e),
                    }
                    let delay = 1000 / freq as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
    });

    // PS Interrupt Subscription
    use_resource(move || {
        let id = app_state.connected_device_id.read().clone();
        async move {
            if id.is_empty() {
                return;
            }
            // Wait for services to be discovered
            for _ in 0..20 {
                if let Ok(services) = ble.list_characteristics(id.clone()).await {
                    if services.iter().any(|s| s.uuid == SERVICE_UUID) {
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            match ble.subscribe(&id, SERVICE_UUID, PS_INT_UUID).await {
                Ok(mut stream) => {
                    while let Some(_data) = stream.next().await {
                        if cancelled() {
                            break;
                        }
                        match parse_interrupt_notification(&_data.value) {
                            Some(true) => {}
                            Some(false) => continue,
                            None => {
                                error!(
                                    "PS interrupt characteristic returned an invalid notification"
                                );
                                continue;
                            }
                        }
                        trigger_count_1.with_mut(|c| *c += 1);
                        trigger_active_1.set(true);
                        let mut active = trigger_active_1;
                        spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                            active.set(false);
                        });
                    }
                }
                Err(e) => error!("Failed to subscribe PS int: {}", e),
            }
        }
    });

    // Reload Interrupt Subscription
    use_resource(move || {
        let id = app_state.connected_device_id.read().clone();
        async move {
            if id.is_empty() {
                return;
            }
            // Wait for services to be discovered
            let mut services_ready = false;
            for _ in 0..20 {
                if let Ok(services) = ble.list_characteristics(id.clone()).await {
                    if services.iter().any(|s| s.uuid == SERVICE_UUID) {
                        services_ready = true;
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            if !services_ready {
                return;
            }
            match ble.subscribe(&id, SERVICE_UUID, RELOAD_INT_UUID).await {
                Ok(mut stream) => {
                    while let Some(_data) = stream.next().await {
                        if cancelled() {
                            break;
                        }
                        match parse_interrupt_notification(&_data.value) {
                            Some(true) => {}
                            Some(false) => continue,
                            None => {
                                error!(
                                    "Reload interrupt characteristic returned an invalid notification"
                                );
                                continue;
                            }
                        }
                        trigger_count_2.with_mut(|c| *c += 1);
                        trigger_active_2.set(true);
                        let mut active = trigger_active_2;
                        spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                            active.set(false);
                        });
                    }
                }
                Err(e) => error!("Failed to subscribe Reload int: {}", e),
            }
        }
    });

    let theme = (app_state.theme)();
    rsx! {
        div {
            class: theme
                .pick(
                    "h-full w-full text-gray-100 overflow-auto relative",
                    "h-full w-full text-gray-900 overflow-auto relative",
                ),
            if device_id.is_empty() {
                StatusEmptyState {}
            } else {
                if !is_loaded() {
                    div {
                        class: theme
                            .pick(
                                "absolute inset-0 z-50 flex items-center justify-center bg-[#1b1b1b]/80 backdrop-blur-sm",
                                "absolute inset-0 z-50 flex items-center justify-center bg-white/80 backdrop-blur-sm",
                            ),
                        div { class: "flex flex-col items-center gap-3",
                            span { class: "animate-spin inline-block w-8 h-8 border-4 border-[#60cd18] border-t-transparent rounded-full" }
                            span { class: theme.pick("text-sm text-gray-300 font-medium", "text-sm text-gray-600 font-medium"),
                                "正在发现服务..."
                            }
                        }
                    }
                }
                StatusContent {
                    battery_level: battery_level(),
                    sensor_data_1: sensor_data_1(),
                    trigger_count_1: trigger_count_1(),
                    trigger_count_2: trigger_count_2(),
                    trigger_active_1: trigger_active_1(),
                    trigger_active_2: trigger_active_2(),
                    ps_freq: ps_refresh_freq(),
                    on_ps_freq_change,
                    on_reset_ps: move |_| trigger_count_1.set(0),
                    on_reset_reload: move |_| trigger_count_2.set(0),
                }
            }
        }
    }
}

fn update_ps_data(
    data: &[u8],
    sensor_data: &mut Signal<Vec<u16>>,
    first_sample: &mut bool,
    invalid_sample_logged: &mut bool,
    source: &str,
) {
    let Some(value) = parse_ps_value(data) else {
        if !*invalid_sample_logged {
            error!(
                "PS {source} returned an invalid value length: {} bytes",
                data.len()
            );
            *invalid_sample_logged = true;
        }
        return;
    };
    if *first_sample {
        info!("Received first PS {source} sample: {data:02X?} = {value} (little-endian)");
        *first_sample = false;
    }
    sensor_data.with_mut(|readings| {
        if readings.len() >= 50 {
            readings.remove(0);
        }
        readings.push(value);
    });
}

fn ps_chart_max(data: &[u16]) -> u16 {
    data.iter()
        .copied()
        .max()
        .unwrap_or(0)
        .max(PS_CHART_BASE_MAX)
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
fn StatusContent(
    battery_level: i32,
    sensor_data_1: Vec<u16>,
    trigger_count_1: i32,
    trigger_count_2: i32,
    trigger_active_1: bool,
    trigger_active_2: bool,
    ps_freq: i32,
    on_ps_freq_change: EventHandler<i32>,
    on_reset_ps: EventHandler<()>,
    on_reset_reload: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "w-full max-w-5xl mx-auto space-y-5",
            StatusHeader { battery_level }
            SensorCharts {
                ps_data: sensor_data_1,
                ps_freq,
                on_ps_freq_change,
            }
            TriggerCounters {
                ps_int_count: trigger_count_1,
                reload_int_count: trigger_count_2,
                ps_int_active: trigger_active_1,
                reload_int_active: trigger_active_2,
                on_reset_ps,
                on_reset_reload,
            }
        }
    }
}

#[component]
fn StatusHeader(battery_level: i32) -> Element {
    let (battery_color, battery_bg) = get_battery_style(battery_level);

    rsx! {
        div { class: "page-header flex items-end justify-between gap-4",
            div {
                p { class: "eyebrow", "LIVE TELEMETRY" }
                h1 { class: "mt-1 text-2xl font-semibold tracking-tight", "设备状态" }
                p { class: "mt-1 text-sm text-slate-400", "实时查看接近传感器数据与设备通知" }
            }
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
fn SensorCharts(ps_data: Vec<u16>, ps_freq: i32, on_ps_freq_change: EventHandler<i32>) -> Element {
    let chart_max = ps_chart_max(&ps_data);
    rsx! {
        div { class: "grid grid-cols-1 gap-4",
            SensorCard {
                title: "PS 传感器数据".to_string(),
                subtitle: "接近传感器实时数据 (UUID: 0xFFE7)".to_string(),
                current_value: ps_data.last().copied(),
                data: ps_data,
                color: "#60cd18".to_string(),
                freq: ps_freq,
                max_value: chart_max,
                on_freq_change: on_ps_freq_change,
            }
        }
    }
}

#[component]
fn SensorCard(
    title: String,
    subtitle: String,
    current_value: Option<u16>,
    data: Vec<u16>,
    color: String,
    freq: i32,
    max_value: u16,
    on_freq_change: EventHandler<i32>,
) -> Element {
    let value_color = if color == "#60cd18" {
        "text-[#60cd18]"
    } else {
        "text-[#2d6cdf]"
    };
    let app_state = use_app_state();
    let theme = (app_state.theme)();

    rsx! {
        div {
            class: theme
                .pick(
                    "panel rounded-2xl border border-white/10 bg-white/[0.04] p-5 space-y-3 shadow-xl shadow-black/10",
                    "panel rounded-2xl border border-slate-200 bg-white p-5 space-y-3 shadow-sm",
                ),
            div {
                div { class: "flex items-center justify-between mb-1",
                    h2 { class: "text-base font-medium", "{title}" }
                    div {
                        class: theme
                            .pick(
                                "flex items-center gap-2 text-xs text-slate-400",
                                "flex items-center gap-2 text-xs text-slate-500",
                            ),
                        span { "刷新" }
                        input {
                            class: theme.pick("sensor-frequency-input sensor-frequency-input-dark", "sensor-frequency-input sensor-frequency-input-light"),
                            r#type: "number",
                            min: "0",
                            max: "15",
                            value: "{freq}",
                            step: "1",
                            inputmode: "numeric",
                            onchange: move |e| {
                                if let Ok(value) = e.value().parse::<i32>() {
                                    on_freq_change.call(value.clamp(0, PS_FREQ_MAX));
                                }
                            },
                        }
                        span { "次/秒" }
                        span { class: "text-[10px] text-slate-500", "0=停止" }
                    }
                }
                p { class: "text-[10px] text-gray-500", "{subtitle}" }
            }
            div { class: theme.pick("sensor-reading sensor-reading-dark", "sensor-reading sensor-reading-light"),
                span { class: "sensor-reading-label", "当前值" }
                div { class: "sensor-reading-value-group",
                    if let Some(value) = current_value {
                        div { class: "sensor-reading-value tabular-nums {value_color}", "{value}" }
                    } else {
                        div { class: "sensor-reading-empty", "等待读取数据" }
                    }
                    div { class: "sensor-reading-scale", "图表范围 0–{max_value}" }
                }
            }
            ChartView { data, color, max_value }
        }
    }
}

#[component]
fn TriggerCounters(
    ps_int_count: i32,
    reload_int_count: i32,
    ps_int_active: bool,
    reload_int_active: bool,
    on_reset_ps: EventHandler<()>,
    on_reset_reload: EventHandler<()>,
) -> Element {
    let app_state = use_app_state();
    let theme = (app_state.theme)();
    rsx! {
        div {
            class: theme
                .pick(
                    "panel rounded-2xl border border-white/10 bg-white/[0.04] p-5",
                    "panel rounded-2xl border border-slate-200 bg-white p-5 shadow-sm",
                ),
            div { class: "mb-3",
                h2 { class: "text-base font-medium", "中断通知计数器" }
                p { class: theme.pick("text-[10px] text-gray-500 mt-0.5", "text-[10px] text-gray-400 mt-0.5"),
                    "接收来自设备的中断通知"
                }
            }
            div { class: "grid grid-cols-1 md:grid-cols-2 gap-4",
                TriggerCounter {
                    label: "PS 中断通知".to_string(),
                    subtitle: "UUID: 0xFFE4".to_string(),
                    count: ps_int_count,
                    is_active: ps_int_active,
                    color: "emerald",
                    on_reset: on_reset_ps,
                }
                TriggerCounter {
                    label: "Reload 中断通知".to_string(),
                    subtitle: "UUID: 0xFFE6".to_string(),
                    count: reload_int_count,
                    is_active: reload_int_active,
                    color: "orange",
                    on_reset: on_reset_reload,
                }
            }
        }
    }
}

#[component]
fn TriggerCounter(
    label: String,
    subtitle: String,
    count: i32,
    is_active: bool,
    color: &'static str,
    on_reset: EventHandler<()>,
) -> Element {
    let (text_color, border_color, bg_color, ripple_color) = get_trigger_colors(color);
    let app_state = use_app_state();
    let theme = (app_state.theme)();
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
                        "{count}"
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
                        onclick: move |_| on_reset.call(()),
                        "重置"
                    }
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
fn ChartView(data: Vec<u16>, color: String, max_value: u16) -> Element {
    let path_data = generate_chart_path(&data, max_value);
    let app_state = use_app_state();
    let theme = (app_state.theme)();

    rsx! {
        div {
            class: theme
                .pick(
                "w-full h-40 bg-[#0b1521] rounded-xl p-3 border border-white/10",
                "w-full h-40 bg-slate-50 rounded-xl p-3 border border-slate-200",
                ),
            svg {
                class: "w-full h-full",
                view_box: "0 0 100 150",
                preserve_aspect_ratio: "none",
                ChartGrid {}
                ChartLine { path: path_data.line_path, color: color.clone() }
                ChartFill { path: path_data.fill_path, color }
            }
        }
    }
}

#[component]
fn ChartGrid() -> Element {
    rsx! {
        for i in 0..5 {
            line {
                x1: "0",
                y1: "{i as f64 * 150.0 / 4.0}",
                x2: "100",
                y2: "{i as f64 * 150.0 / 4.0}",
                stroke: "#33475a",
                stroke_width: "0.5",
            }
        }
    }
}

#[component]
fn ChartLine(path: String, color: String) -> Element {
    rsx! {
        path {
            d: "{path}",
            fill: "none",
            stroke: "{color}",
            stroke_width: "2",
            stroke_linejoin: "round",
            stroke_linecap: "round",
        }
    }
}

#[component]
fn ChartFill(path: String, color: String) -> Element {
    rsx! {
        path { d: "{path}", fill: "{color}", fill_opacity: "0.1" }
    }
}

struct ChartPath {
    line_path: String,
    fill_path: String,
}

fn generate_chart_path(data: &[u16], max_value: u16) -> ChartPath {
    let chart_height = 150.0;
    let chart_width = 100.0;
    let data_points = data.len();

    if data_points == 0 || max_value == 0 {
        return ChartPath {
            line_path: format!("M 0 {chart_height}"),
            fill_path: format!("M 0 {chart_height} L {chart_width} {chart_height} Z"),
        };
    }

    let mut path_parts: Vec<String> = Vec::new();

    for (i, value) in data.iter().enumerate() {
        let x = if data_points == 1 {
            chart_width / 2.0
        } else {
            (i as f64 / (data_points - 1) as f64) * chart_width
        };
        let normalized = (*value as f64 / max_value as f64).clamp(0.0, 1.0);
        let y = chart_height - (normalized * chart_height);

        if i == 0 {
            path_parts.push(format!("M {:.2} {:.2}", x, y));
        } else {
            path_parts.push(format!("L {:.2} {:.2}", x, y));
        }
    }

    let line_path = path_parts.join(" ");
    let fill_path = format!(
        "{} L {:.2} {:.2} L 0 {:.2} Z",
        line_path, chart_width, chart_height, chart_height
    );

    ChartPath {
        line_path,
        fill_path,
    }
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
        parse_battery_level, parse_interrupt_notification, parse_ps_frequency, parse_ps_value,
        ps_chart_max,
    };

    #[test]
    fn ps_reading_requires_exactly_two_little_endian_bytes() {
        assert_eq!(parse_ps_value(&[0x34, 0x12]), Some(0x1234));
        assert_eq!(parse_ps_value(&[0x2a]), None);
        assert_eq!(parse_ps_value(&[]), None);
        assert_eq!(parse_ps_value(&[0x34, 0x12, 0]), None);
    }

    #[test]
    fn chart_scale_expands_when_a_reading_exceeds_twelve_bits() {
        assert_eq!(ps_chart_max(&[0, 100, 4095]), 4095);
        assert_eq!(ps_chart_max(&[0, 0x1234]), 0x1234);
        assert_eq!(ps_chart_max(&[]), 4095);
    }

    #[test]
    fn ps_frequency_uses_the_low_nibble_and_zero_means_stopped() {
        assert_eq!(parse_ps_frequency(&[0]), Some(0));
        assert_eq!(parse_ps_frequency(&[15]), Some(15));
        assert_eq!(parse_ps_frequency(&[0xf5]), Some(5));
        assert_eq!(parse_ps_frequency(&[]), None);
        assert_eq!(parse_ps_frequency(&[1, 2]), None);
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
