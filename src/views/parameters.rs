use crate::api::ble_service::use_ble;
use crate::ble_constants::{DEVICE_NAME_UUID, PS_HIGH_UUID, PS_LOW_UUID, SERVICE_UUID};
use crate::context::use_app_state;
use dioxus::prelude::*;

#[component]
pub fn Parameters() -> Element {
    let mut app_state = use_app_state();
    let ble = use_ble();
    let device_id = app_state.connected_device_id.read().clone();

    // 参数状态
    let mut threshold_low = use_signal(|| 500);
    let mut threshold_high = use_signal(|| 3000);
    let mut device_name = use_signal(String::new);
    let mut saved_device_name = use_signal(String::new);
    let mut has_changes = use_signal(|| false);
    let mut is_loaded = use_signal(|| false);
    let mut is_saving = use_signal(|| false);
    let mut is_saving_device_name = use_signal(|| false);
    let mut loaded_device_id = use_signal(String::new);

    // 读取初始参数
    use_effect(move || {
        let id = app_state.connected_device_id.read().clone();
        if id.is_empty() {
            loaded_device_id.set(String::new());
            is_loaded.set(false);
            has_changes.set(false);
            device_name.set(String::new());
            saved_device_name.set(String::new());
            return;
        }

        if loaded_device_id() == id {
            return;
        }

        loaded_device_id.set(id.clone());
        is_loaded.set(false);
        has_changes.set(false);
        threshold_low.set(500);
        threshold_high.set(3000);
        device_name.set(String::new());
        saved_device_name.set(String::new());
        let fallback_name = app_state
            .scanned_devices
            .read()
            .iter()
            .find(|device| device.id == id)
            .map(|device| device.name.clone());

        spawn(async move {
            let mut low = 500;
            let mut high = 3000;
            let mut services_ready = false;

            // Wait for services to be discovered
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
                if app_state.connected_device_id.read().as_str() == id {
                    is_loaded.set(true);
                }
                return;
            }

            // 读取 PS Low (0xFFE2)
            if let Ok(data) = ble.read(&id, SERVICE_UUID, PS_LOW_UUID).await {
                if let Some(value) = parse_threshold_value(&data) {
                    low = value;
                }
            }

            // 读取 PS High (0xFFE1)
            if let Ok(data) = ble.read(&id, SERVICE_UUID, PS_HIGH_UUID).await {
                if let Some(value) = parse_threshold_value(&data) {
                    high = value;
                }
            }

            let name = ble
                .read(&id, SERVICE_UUID, DEVICE_NAME_UUID)
                .await
                .ok()
                .and_then(|data| String::from_utf8(data).ok())
                .filter(|name| device_name_validation_error(name).is_none())
                .or(fallback_name);

            if app_state.connected_device_id.read().as_str() == id {
                let (low, high) = normalize_thresholds(low, high);
                threshold_low.set(low);
                threshold_high.set(high);
                if let Some(name) = name {
                    device_name.set(name.clone());
                    saved_device_name.set(name);
                }
                is_loaded.set(true);
            }
        });
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
                ParametersEmptyState {}
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
                                "正在读取参数..."
                            }
                        }
                    }
                }
                ParametersContent {
                    threshold_low: threshold_low(),
                    threshold_high: threshold_high(),
                    device_name: device_name(),
                    name_has_changes: device_name() != saved_device_name(),
                    has_changes: has_changes(),
                    is_saving: is_saving(),
                    is_saving_device_name: is_saving_device_name(),
                    on_device_name_change: move |name: String| device_name.set(name),
                    on_save_device_name: move |_| {
                        if is_saving_device_name() {
                            return;
                        }
                        let name = device_name();
                        if let Some(reason) = device_name_validation_error(&name) {
                            error!("Cannot save device name: {reason}");
                            return;
                        }
                        let id = app_state.connected_device_id.read().clone();
                        if id.is_empty() {
                            return;
                        }
                        is_saving_device_name.set(true);
                        spawn(async move {
                            match ble
                                .write(
                                    &id,
                                    SERVICE_UUID,
                                    DEVICE_NAME_UUID,
                                    name.as_bytes().to_vec(),
                                )
                                .await
                            {
                                Ok(()) => {
                                    if app_state.connected_device_id.read().as_str() == id {
                                        saved_device_name.set(name.clone());
                                    }
                                    app_state.scanned_devices.with_mut(|devices| {
                                        if let Some(device) = devices.iter_mut().find(|device| device.id == id) {
                                            device.name = name;
                                        }
                                    });
                                }
                                Err(error) => error!("Failed to save device name: {error}"),
                            }
                            is_saving_device_name.set(false);
                        });
                    },
                    on_save: move |_| {
                        if is_saving() {
                            return;
                        }
                        let id = app_state.connected_device_id.read().clone();
                        if id.is_empty() {
                            error!("Cannot save parameters without a connected device");
                            return;
                        }
                        let (low, high) = normalize_thresholds(threshold_low(), threshold_high());
                        threshold_low.set(low);
                        threshold_high.set(high);
                        is_saving.set(true);
                        spawn(async move {
                            let result = async {
                                ble
                                .write(
                                    &id,
                                    SERVICE_UUID,
                                    PS_LOW_UUID,
                                    (low as u16).to_le_bytes().to_vec(),
                                )
                                .await?;
                                ble
                                .write(
                                    &id,
                                    SERVICE_UUID,
                                    PS_HIGH_UUID,
                                    (high as u16).to_le_bytes().to_vec(),
                                )
                                .await
                            }
                            .await;
                            match result {
                                Ok(())
                                    if app_state.connected_device_id.read().as_str() == id
                                        && threshold_low() == low
                                        && threshold_high() == high =>
                                {
                                    has_changes.set(false)
                                }
                                Ok(()) => {}
                                Err(error) => error!("Failed to save parameters: {error}"),
                            }
                            is_saving.set(false);
                        });
                    },
                    on_reset: move |_| {
                        // 重新读取参数
                        let id = app_state.connected_device_id.read().clone();
                        if id.is_empty() {
                            return;
                        }
                        spawn(async move {
                            let result = async {
                                let low_data = ble.read(&id, SERVICE_UUID, PS_LOW_UUID).await?;
                                let low = parse_threshold_value(&low_data)
                                    .ok_or_else(|| "PS low threshold returned an invalid value".to_string())?;
                                let high_data = ble.read(&id, SERVICE_UUID, PS_HIGH_UUID).await?;
                                let high = parse_threshold_value(&high_data)
                                    .ok_or_else(|| "PS high threshold returned an invalid value".to_string())?;
                                Ok::<_, String>(normalize_thresholds(low, high))
                            }
                            .await;
                            match result {
                                Ok((low, high)) if app_state.connected_device_id.read().as_str() == id => {
                                    threshold_low.set(low);
                                    threshold_high.set(high);
                                    has_changes.set(false);
                                }
                                Ok(_) => {}
                                Err(error) => error!("Failed to reload parameters: {error}"),
                            }
                        });
                    },
                    on_low_change: move |value: i32| {
                        threshold_low.set(value.clamp(0, threshold_high() - 1));
                        has_changes.set(true);
                    },
                    on_high_change: move |value: i32| {
                        threshold_high.set(value.clamp(threshold_low() + 1, MAX_THRESHOLD));
                        has_changes.set(true);
                    },
                }
            }
        }
    }
}

#[component]
fn ParametersEmptyState() -> Element {
    let theme = *use_app_state().theme.read();
    rsx! {
        div { class: "flex items-center justify-center h-full min-h-[360px]",
            div { class: theme.pick("empty-panel max-w-sm rounded-2xl border border-white/10 bg-white/[0.03] px-8 py-10 text-center shadow-xl", "empty-panel max-w-sm rounded-2xl border border-slate-200 bg-white px-8 py-10 text-center shadow-sm"),
                div { class: "mx-auto flex h-14 w-14 items-center justify-center rounded-2xl bg-[#1d3040] text-2xl text-[#b8f36b]", "↯" }
                div { class: theme.pick("mt-5 text-lg font-semibold text-slate-200", "mt-5 text-lg font-semibold text-slate-800"), "未连接设备" }
                div { class: theme.pick("mt-2 text-sm text-slate-400", "mt-2 text-sm text-slate-500"), "请先前往连接页面建立 BLE 连接" }
            }
        }
    }
}

#[component]
fn ParametersContent(
    threshold_low: i32,
    threshold_high: i32,
    device_name: String,
    name_has_changes: bool,
    has_changes: bool,
    is_saving: bool,
    is_saving_device_name: bool,
    on_save: EventHandler<()>,
    on_reset: EventHandler<()>,
    on_low_change: EventHandler<i32>,
    on_high_change: EventHandler<i32>,
    on_device_name_change: EventHandler<String>,
    on_save_device_name: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "w-full max-w-5xl mx-auto space-y-5",
            ParametersHeader { has_changes, is_saving, on_save, on_reset }

            ThresholdPairCard {
                low_value: threshold_low,
                high_value: threshold_high,
                on_low_change,
                on_high_change,
            }

            DeviceNameCard {
                value: device_name,
                has_changes: name_has_changes,
                is_saving: is_saving_device_name,
                on_change: on_device_name_change,
                on_save: on_save_device_name,
            }
        }
    }
}

#[component]
fn DeviceNameCard(
    value: String,
    has_changes: bool,
    is_saving: bool,
    on_change: EventHandler<String>,
    on_save: EventHandler<()>,
) -> Element {
    let app_state = use_app_state();
    let theme = (app_state.theme)();
    let validation_error = device_name_validation_error(&value);
    let can_save = has_changes && !is_saving && validation_error.is_none();
    let save_class = if can_save {
        "inline-flex items-center rounded-lg bg-[#60cd18] hover:bg-[#6fe12a] cursor-pointer px-3 py-1.5 text-xs font-medium text-gray-900 transition-colors"
    } else {
        theme.pick(
            "inline-flex items-center rounded-lg bg-[#2a2a2a] px-3 py-1.5 text-xs font-medium text-gray-500 cursor-not-allowed",
            "inline-flex items-center rounded-lg bg-gray-100 px-3 py-1.5 text-xs font-medium text-gray-400 cursor-not-allowed",
        )
    };

    rsx! {
        div {
            class: theme
                .pick(
                    "panel rounded-2xl border border-white/10 bg-white/[0.04] p-5 space-y-4 shadow-xl shadow-black/10",
                    "panel rounded-2xl border border-slate-200 bg-white p-5 space-y-4 shadow-sm",
                ),
            div { class: "flex items-start justify-between gap-4",
                div {
                    h2 { class: "text-base font-semibold mb-1", "蓝牙设备名称" }
                    p { class: "text-xs text-gray-400", "更新设备广播名称，最多 20 个 UTF-8 字节" }
                }
                button {
                    class: "{save_class}",
                    disabled: !can_save,
                    onclick: move |_| on_save.call(()),
                    if is_saving { "保存中..." } else { "保存名称" }
                }
            }
            input {
                class: theme.pick("device-name-input device-name-input-dark", "device-name-input device-name-input-light"),
                r#type: "text",
                value: "{value}",
                autocomplete: "off",
                spellcheck: "false",
                aria_label: "蓝牙设备名称",
                oninput: move |event| on_change.call(event.value()),
            }
            div { class: "flex items-center justify-between gap-3 text-xs",
                span {
                    class: if validation_error.is_some() { "text-red-400" } else { "text-gray-500" },
                    if let Some(error) = validation_error {
                        "{error}"
                    } else {
                        "名称符合设备格式要求"
                    }
                }
                span { class: "text-gray-500 tabular-nums", "{value.len()}/20 字节" }
            }
        }
    }
}

fn device_name_validation_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("名称不能为空")
    } else if name.len() > 20 {
        Some("名称不能超过 20 个 UTF-8 字节")
    } else if name.chars().any(|character| character.is_ascii_control()) {
        Some("名称不能包含 NUL 或 ASCII 控制字符")
    } else {
        None
    }
}

#[component]
fn ParametersHeader(
    has_changes: bool,
    is_saving: bool,
    on_save: EventHandler<()>,
    on_reset: EventHandler<()>,
) -> Element {
    let app_state = use_app_state();
    let theme = (app_state.theme)();
    let reset_btn_class = theme.pick(
        "inline-flex items-center gap-2 rounded-lg bg-[#2a2a2a] hover:bg-[#333] cursor-pointer px-3 py-1.5 text-xs font-medium text-gray-300 transition-colors",
        "inline-flex items-center gap-2 rounded-lg bg-gray-100 hover:bg-gray-200 cursor-pointer px-3 py-1.5 text-xs font-medium text-gray-700 transition-colors",
    );
    let save_active_class = "inline-flex items-center gap-2 rounded-lg bg-[#60cd18] hover:bg-[#6fe12a] cursor-pointer px-3 py-1.5 text-xs font-medium text-gray-900 transition-colors";
    let save_inactive_class = theme.pick(
        "inline-flex items-center gap-2 rounded-lg bg-[#2a2a2a] px-3 py-1.5 text-xs font-medium text-gray-500 cursor-not-allowed",
        "inline-flex items-center gap-2 rounded-lg bg-gray-100 px-3 py-1.5 text-xs font-medium text-gray-400 cursor-not-allowed",
    );
    rsx! {
        div { class: "page-header flex flex-col md:flex-row md:items-end md:justify-between gap-4",
            div {
                p { class: "eyebrow", "DEVICE CONFIGURATION" }
                h1 { class: "mt-1 text-2xl font-semibold tracking-tight", "参数设置" }
                p { class: "mt-1 text-sm text-slate-400", "调整接近传感器的触发区间" }
                if has_changes {
                    p { class: "text-xs text-yellow-400 mt-0.5", "● 有未保存的更改" }
                }
            }
            div { class: "flex gap-2",
                button {
                    class: "{reset_btn_class}",
                    onclick: move |_| on_reset.call(()),
                    "重置"
                }
                button {
                    class: if has_changes { save_active_class } else { save_inactive_class },
                    disabled: !has_changes || is_saving,
                    onclick: move |_| on_save.call(()),
                    if is_saving { "保存中..." } else { "保存" }
                }
            }
        }
    }
}

#[component]
fn ThresholdPairCard(
    low_value: i32,
    high_value: i32,
    on_low_change: EventHandler<i32>,
    on_high_change: EventHandler<i32>,
) -> Element {
    let low_value = low_value.clamp(0, MAX_THRESHOLD - 1);
    let high_value = high_value.clamp(low_value + 1, MAX_THRESHOLD);
    let percentage_low = low_value as f64 / MAX_THRESHOLD as f64 * 100.0;
    let percentage_high = high_value as f64 / MAX_THRESHOLD as f64 * 100.0;
    let app_state = use_app_state();
    let theme = (app_state.theme)();
    let low_value_class = theme.pick(
        "threshold-value-card threshold-value-dark",
        "threshold-value-card threshold-value-light",
    );
    let low_input_class = theme.pick(
        "threshold-number threshold-number-dark text-blue-300",
        "threshold-number threshold-number-light text-blue-700",
    );
    let high_input_class = theme.pick(
        "threshold-number threshold-number-dark text-red-300",
        "threshold-number threshold-number-light text-red-700",
    );

    rsx! {
        div {
            class: theme
                .pick(
                    "panel rounded-2xl border border-white/10 bg-white/[0.04] p-5 space-y-5 shadow-xl shadow-black/10",
                    "panel rounded-2xl border border-slate-200 bg-white p-5 space-y-5 shadow-sm",
                ),
            div {
                h2 { class: "text-base font-semibold mb-1", "接近传感器阈值" }
                p { class: "text-xs text-gray-400",
                    "拖动两侧滑块，调整传感器的触发区间"
                }
            }

            div { class: "threshold-value-group",
                label { class: "{low_value_class}",
                    span { class: "threshold-value-label",
                        span { class: "threshold-value-dot threshold-dot-low" }
                        "低阈值"
                    }
                    input {
                        r#type: "number",
                        min: "0",
                        max: "{high_value - 1}",
                        step: "1",
                        inputmode: "numeric",
                        value: "{low_value}",
                        class: "{low_input_class}",
                        aria_label: "低阈值",
                        onchange: move |event| {
                            if let Ok(value) = event.value().parse::<i32>() {
                                on_low_change.call(value.clamp(0, high_value - 1));
                            }
                        },
                    }
                }
                label { class: "{low_value_class}",
                    span { class: "threshold-value-label",
                        span { class: "threshold-value-dot threshold-dot-high" }
                        "高阈值"
                    }
                    input {
                        r#type: "number",
                        min: "{low_value + 1}",
                        max: "{MAX_THRESHOLD}",
                        step: "1",
                        inputmode: "numeric",
                        value: "{high_value}",
                        class: "{high_input_class}",
                        aria_label: "高阈值",
                        onchange: move |event| {
                            if let Ok(value) = event.value().parse::<i32>() {
                                on_high_change.call(value.clamp(low_value + 1, MAX_THRESHOLD));
                            }
                        },
                    }
                }
            }

            div { class: "px-2 pt-2",
                div { class: "threshold-dual-slider",
                    div {
                        class: theme.pick(
                            "threshold-slider-track threshold-slider-track-dark",
                            "threshold-slider-track threshold-slider-track-light",
                        ),
                    }
                    div {
                        class: "threshold-slider-selection",
                        style: "left: {percentage_low}%; width: {percentage_high - percentage_low}%;",
                    }
                    input {
                        r#type: "range",
                        min: "0",
                        // Both thumbs use the complete sensor range. Restricting
                        // the HTML max to the other thumb makes the browser map
                        // the thumb position to a different scale on every
                        // render, which is why the old slider appeared to jump.
                        max: "{MAX_THRESHOLD}",
                        value: "{low_value}",
                        class: theme.pick(
                            "threshold-range threshold-range-low threshold-range-dark",
                            "threshold-range threshold-range-low threshold-range-light",
                        ),
                        aria_label: "低阈值",
                        aria_valuemin: "0",
                        aria_valuemax: "{MAX_THRESHOLD}",
                        aria_valuenow: "{low_value}",
                        oninput: move |event| {
                            if let Ok(value) = event.value().parse::<i32>() {
                                on_low_change.call(value.min(high_value - 1));
                            }
                        },
                    }
                    input {
                        r#type: "range",
                        min: "0",
                        max: "{MAX_THRESHOLD}",
                        value: "{high_value}",
                        class: theme.pick(
                            "threshold-range threshold-range-high threshold-range-dark",
                            "threshold-range threshold-range-high threshold-range-light",
                        ),
                        aria_label: "高阈值",
                        aria_valuemin: "0",
                        aria_valuemax: "{MAX_THRESHOLD}",
                        aria_valuenow: "{high_value}",
                        oninput: move |event| {
                            if let Ok(value) = event.value().parse::<i32>() {
                                on_high_change.call(value.max(low_value + 1));
                            }
                        },
                    }
                }
                div { class: theme.pick(
                    "threshold-scale threshold-scale-dark",
                    "threshold-scale threshold-scale-light",
                ),
                    span { "0" }
                    span { "32,768" }
                    span { "65,535" }
                }
            }
        }
    }
}

const MAX_THRESHOLD: i32 = u16::MAX as i32;

fn normalize_thresholds(low: i32, high: i32) -> (i32, i32) {
    let low = low.clamp(0, MAX_THRESHOLD - 1);
    let high = high.clamp(low + 1, MAX_THRESHOLD);
    (low, high)
}

fn parse_threshold_value(data: &[u8]) -> Option<i32> {
    // Both threshold characteristics are two-byte little-endian values. Do
    // not mask the high nibble here because a valid device value such as
    // 0x1000 would otherwise be displayed as zero.
    match data {
        [low, high] => Some(u16::from_le_bytes([*low, *high]) as i32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        device_name_validation_error, normalize_thresholds, parse_threshold_value, MAX_THRESHOLD,
    };

    #[test]
    fn threshold_values_keep_the_full_u16_range() {
        assert_eq!(parse_threshold_value(&[0x34, 0x12]), Some(0x1234));
        assert_eq!(parse_threshold_value(&[0xff, 0xff]), Some(MAX_THRESHOLD));
        assert_eq!(parse_threshold_value(&[0x12]), None);
        assert_eq!(parse_threshold_value(&[0x34, 0x12, 0]), None);
    }

    #[test]
    fn threshold_pair_stays_ordered_at_range_edges() {
        assert_eq!(normalize_thresholds(-1, 0), (0, 1));
        assert_eq!(normalize_thresholds(100, 50), (100, 101));
        assert_eq!(
            normalize_thresholds(70_000, 80_000),
            (MAX_THRESHOLD - 1, MAX_THRESHOLD)
        );
    }

    #[test]
    fn device_name_obeys_utf8_byte_and_control_character_limits() {
        assert_eq!(device_name_validation_error("Sensor-01"), None);
        assert_eq!(device_name_validation_error("传感器"), None);
        assert_eq!(device_name_validation_error(""), Some("名称不能为空"));
        assert_eq!(
            device_name_validation_error("传感器名称测试"),
            Some("名称不能超过 20 个 UTF-8 字节")
        );
        assert_eq!(
            device_name_validation_error("Sensor\n01"),
            Some("名称不能包含 NUL 或 ASCII 控制字符")
        );
    }
}
