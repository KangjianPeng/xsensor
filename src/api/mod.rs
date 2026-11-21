pub mod ble;
pub mod ble_service;

#[cfg(not(target_arch = "wasm32"))]
use btleplug::api::{CharPropFlags, Characteristic};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UiDevice {
    pub id: String,
    pub name: String,
    pub is_connected: bool,
    pub rssi: Option<i16>,
    pub services: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiService {
    pub uuid: String,
    pub name: String,
    pub characteristic: Vec<UiCharacteristic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiCharacteristic {
    pub uuid: String,
    pub name: String,
    pub service_uuid: String,
    pub property: Vec<String>,
}

pub fn characteristic_name(uuid: &str) -> &str {
    match uuid {
        "0000ffe1-0000-1000-8000-00805f9b34fb" => "PS High Threshold",
        "0000ffe2-0000-1000-8000-00805f9b34fb" => "PS Low Threshold",
        "0000ffe4-0000-1000-8000-00805f9b34fb" => "PS Interrupt",
        "0000ffe6-0000-1000-8000-00805f9b34fb" => "Reload Interrupt",
        "0000ffe7-0000-1000-8000-00805f9b34fb" => "PS Sensor Value",
        "0000ffe9-0000-1000-8000-00805f9b34fb" => "PS Refresh Frequency",
        "0000ffea-0000-1000-8000-00805f9b34fb" => "BLE Device Name",
        "00002a19-0000-1000-8000-00805f9b34fb" => "Battery Level",
        _ => "Unknown Characteristic",
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn prop_flags_to_vec(flags: CharPropFlags) -> Vec<String> {
    let mut out = Vec::new();
    if flags.contains(CharPropFlags::READ) {
        out.push("read".into());
    }
    if flags.contains(CharPropFlags::WRITE) {
        out.push("write".into());
    }
    if flags.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
        out.push("write_without_response".into());
    }
    if flags.contains(CharPropFlags::NOTIFY) {
        out.push("notify".into());
    }
    if flags.contains(CharPropFlags::INDICATE) {
        out.push("indicate".into());
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
impl From<Characteristic> for UiCharacteristic {
    fn from(value: Characteristic) -> Self {
        UiCharacteristic {
            uuid: value.uuid.to_string(),
            name: characteristic_name(&value.uuid.to_string()).to_string(),
            service_uuid: value.service_uuid.to_string(),
            property: prop_flags_to_vec(value.properties),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationData {
    pub uuid: String,
    pub value: Vec<u8>,
}
