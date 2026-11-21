/// 主要传感器服务 UUID
pub const SERVICE_UUID: &str = "0000ffe0-0000-1000-8000-00805f9b34fb";

/// 接近传感器阈值特征 UUID
pub const PS_HIGH_UUID: &str = "0000ffe1-0000-1000-8000-00805f9b34fb";
pub const PS_LOW_UUID: &str = "0000ffe2-0000-1000-8000-00805f9b34fb";

/// 中断通知特征 UUID
pub const PS_INT_UUID: &str = "0000ffe4-0000-1000-8000-00805f9b34fb";
pub const RELOAD_INT_UUID: &str = "0000ffe6-0000-1000-8000-00805f9b34fb";

/// 数据特征 UUID
pub const PS_DATA_UUID: &str = "0000ffe7-0000-1000-8000-00805f9b34fb";
pub const PS_FREQ_UUID: &str = "0000ffe9-0000-1000-8000-00805f9b34fb";
pub const DEVICE_NAME_UUID: &str = "0000ffea-0000-1000-8000-00805f9b34fb";

/// CHAR9 accepts only the low four bits. Zero disables periodic PS reads.
pub const PS_FREQ_MAX: i32 = 15;

/// 电池服务和特征 UUID
pub const BATTERY_SERVICE_UUID: &str = "0000180f-0000-1000-8000-00805f9b34fb";
pub const BATTERY_LEVEL_UUID: &str = "00002a19-0000-1000-8000-00805f9b34fb";
