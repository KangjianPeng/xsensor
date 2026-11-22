use crate::api::{characteristic_name, NotificationData, UiCharacteristic, UiDevice, UiService};
use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Adapter, Manager, Peripheral};
use dioxus::prelude::info;
use futures::StreamExt;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

// Simple error type
#[derive(Debug)]
pub struct BleError(String);
impl std::fmt::Display for BleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for BleError {}
impl BleError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

pub struct BleManager {
    adapter: Option<Adapter>,
    known_peripherals: HashMap<String, Arc<Peripheral>>,
    connected_peripherals: HashMap<String, Arc<Peripheral>>,
    is_scanning: bool,
}

impl BleManager {
    pub fn new() -> Self {
        Self {
            adapter: None,
            known_peripherals: HashMap::new(),
            connected_peripherals: HashMap::new(),
            is_scanning: false,
        }
    }

    pub async fn ensure_adapter(&mut self) -> Result<Adapter, BleError> {
        if let Some(s) = &self.adapter {
            return Ok(s.clone());
        }
        let manager = Manager::new()
            .await
            .map_err(|e| BleError::new(e.to_string()))?;
        let adpaters = manager
            .adapters()
            .await
            .map_err(|e| BleError::new(e.to_string()))?;
        let adapter = adpaters
            .into_iter()
            .nth(0)
            .ok_or(BleError::new("No Bluetooth adapter found"))?;
        self.adapter = Some(adapter.clone());
        Ok(adapter)
    }

    async fn fetch_devices(adapter: &Adapter) -> Vec<UiDevice> {
        let peripherals = adapter.peripherals().await.unwrap_or_default();
        info!("Fetched {} peripherals", peripherals.len());
        let mut p_list = Vec::new();
        for p in peripherals {
            let props = p.properties().await.unwrap_or(None);
            let name = props
                .as_ref()
                .and_then(|x| x.local_name.clone())
                .unwrap_or_else(|| "未知设备".to_string());
            let rssi = props.as_ref().and_then(|x| x.rssi);
            let services = props
                .as_ref()
                .map(|x| x.services.iter().map(|u| u.to_string()).collect())
                .unwrap_or_default();
            let is_connected = p.is_connected().await.unwrap_or_default();

            p_list.push(UiDevice {
                id: p.id().to_string(),
                name,
                is_connected,
                rssi,
                services,
            });
        }

        // Keep device rows in one deterministic order. Connection state and RSSI
        // change while scanning and must not move a row under the pointer.
        p_list.sort_by(|a, b| a.id.cmp(&b.id));
        p_list
    }

    pub async fn start_scan_stream(
        &mut self,
    ) -> Result<impl futures::Stream<Item = Vec<UiDevice>>, BleError> {
        let adapter = self.ensure_adapter().await?;
        info!("Starting scan on adapter");
        adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| BleError::new(e.to_string()))?;
        info!("Scan started");

        let events = adapter
            .events()
            .await
            .map_err(|e| BleError::new(e.to_string()))?;

        let adapter_clone = adapter.clone();

        let mut refresh_interval = tokio::time::interval(std::time::Duration::from_secs(1));
        refresh_interval.tick().await;
        let updates = futures::stream::unfold(
            (events, refresh_interval),
            |(mut events, mut refresh_interval)| async move {
                loop {
                    tokio::select! {
                        _ = refresh_interval.tick() => break,
                        Some(event) = events.next() => {
                            if let CentralEvent::DeviceDiscovered(_) = event {
                                info!("BLE Event: DeviceDiscovered");
                            }
                        }
                    }
                }
                Some(((), (events, refresh_interval)))
            },
        );

        let stream = updates.then(move |_| {
            let adapter = adapter_clone.clone();
            async move { Self::fetch_devices(&adapter).await }
        });

        // Fetch immediately once
        let initial_adapter = adapter.clone();
        let initial =
            futures::stream::once(async move { Self::fetch_devices(&initial_adapter).await });

        self.is_scanning = true;
        Ok(initial.chain(stream))
    }

    pub async fn stop_scan(&mut self) -> Result<(), BleError> {
        if self.is_scanning {
            if let Some(adapter) = &self.adapter {
                adapter
                    .stop_scan()
                    .await
                    .map_err(|e| BleError::new(e.to_string()))?;
            }
            self.is_scanning = false;
        }
        Ok(())
    }

    pub async fn get_devices(&mut self) -> Result<Vec<UiDevice>, BleError> {
        let adapter = self.ensure_adapter().await?;
        let peripherals = adapter
            .peripherals()
            .await
            .map_err(|e| BleError::new(e.to_string()))?;

        for p in peripherals {
            self.known_peripherals
                .insert(p.id().to_string(), std::sync::Arc::new(p));
        }

        let mut p_list = Vec::new();
        for (id, p) in &self.known_peripherals {
            let props = p.properties().await.unwrap_or(None);
            let name = props
                .as_ref()
                .and_then(|x| x.local_name.clone())
                .unwrap_or_else(|| "未知设备".to_string());
            let rssi = props.as_ref().and_then(|x| x.rssi);
            let services = props
                .as_ref()
                .map(|x| x.services.iter().map(|u| u.to_string()).collect())
                .unwrap_or_default();
            let is_connected = p.is_connected().await.unwrap_or_default();

            p_list.push(UiDevice {
                id: id.clone(),
                name,
                is_connected,
                rssi,
                services,
            });
        }

        // Keep device rows in one deterministic order. Connection state and RSSI
        // change while scanning and must not move a row under the pointer.
        p_list.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(p_list)
    }

    pub async fn connect(&mut self, id: String) -> Result<(), BleError> {
        let adapter = self.ensure_adapter().await?;
        let peripherals = adapter
            .peripherals()
            .await
            .map_err(|e| BleError::new(e.to_string()))?;

        let p = peripherals
            .into_iter()
            .find(|p| p.id().to_string() == id)
            .map(Arc::new)
            .or_else(|| self.known_peripherals.get(&id).cloned())
            .ok_or(BleError::new("Device not found"))?;

        p.connect()
            .await
            .map_err(|e| BleError::new(e.to_string()))?;

        // Complete discovery before exposing the connection to the views. This
        // prevents the first PS reads from racing the Windows service cache.
        if let Err(error) = p.discover_services().await {
            let cleanup_error = p.disconnect().await.err();
            let message = match cleanup_error {
                Some(cleanup_error) => {
                    format!("Service discovery failed: {error}; disconnect failed: {cleanup_error}")
                }
                None => format!("Service discovery failed: {error}"),
            };
            return Err(BleError::new(message));
        }

        self.known_peripherals.insert(id.clone(), p.clone());
        self.connected_peripherals.insert(id, p);

        Ok(())
    }

    pub async fn disconnect(&mut self, id: String) -> Result<(), BleError> {
        if let Some(p) = self.connected_peripherals.get(&id) {
            p.disconnect()
                .await
                .map_err(|e| BleError::new(e.to_string()))?;
            self.connected_peripherals.remove(&id);
        }
        Ok(())
    }

    async fn connected_peripheral(&mut self, id: &str) -> Result<Arc<Peripheral>, BleError> {
        let p = self
            .connected_peripherals
            .get(id)
            .cloned()
            .ok_or_else(|| BleError::new("Device not connected"))?;

        if p.is_connected().await.unwrap_or(false) {
            return Ok(p);
        }

        self.connected_peripherals.remove(id);
        // Release GATT handles after link loss so only a manual connect starts a new session.
        let _ = p.disconnect().await;
        Err(BleError::new("Device not connected"))
    }

    async fn get_characteristic(
        &mut self,
        peri_id: &str,
        service_uuid: &str,
        char_uuid: &str,
    ) -> Result<(Arc<Peripheral>, btleplug::api::Characteristic), BleError> {
        let p = self.connected_peripheral(peri_id).await?;

        let service_uuid_obj = Uuid::from_str(service_uuid)
            .map_err(|e| BleError::new(format!("Invalid service UUID: {}", e)))?;
        let char_uuid_obj = Uuid::from_str(char_uuid)
            .map_err(|e| BleError::new(format!("Invalid characteristic UUID: {}", e)))?;

        let chars = p.characteristics();
        if let Some(c) = chars
            .into_iter()
            .find(|c| c.service_uuid == service_uuid_obj && c.uuid == char_uuid_obj)
        {
            return Ok((p, c));
        }
        Err(BleError::new(format!(
            "Characteristic not found: {}",
            char_uuid,
        )))
    }

    async fn refresh_peripheral(&mut self, peri_id: &str) -> Result<(), BleError> {
        self.connected_peripheral(peri_id).await?;
        let adapter = self.ensure_adapter().await?;
        let peripherals = adapter
            .peripherals()
            .await
            .map_err(|e| BleError::new(e.to_string()))?;

        if let Some(fresh_p) = peripherals
            .into_iter()
            .find(|p| p.id().to_string() == peri_id)
        {
            let fresh_p = Arc::new(fresh_p);
            // 检查是否已连接
            if fresh_p.is_connected().await.unwrap_or(false) {
                // 确保服务已发现
                if fresh_p.services().is_empty() {
                    fresh_p
                        .discover_services()
                        .await
                        .map_err(|e| BleError::new(e.to_string()))?;
                }
                self.known_peripherals
                    .insert(peri_id.to_string(), fresh_p.clone());
                self.connected_peripherals
                    .insert(peri_id.to_string(), fresh_p);
                return Ok(());
            }
        }
        Err(BleError::new("Device not found or disconnected"))
    }

    pub async fn read(
        &mut self,
        peri_id: &str,
        service_uuid: &str,
        char_uuid: &str,
    ) -> Result<Vec<u8>, BleError> {
        let (p, char) = match self
            .get_characteristic(peri_id, service_uuid, char_uuid)
            .await
        {
            Ok(res) => res,
            Err(_) => {
                self.refresh_peripheral(peri_id).await?;
                self.get_characteristic(peri_id, service_uuid, char_uuid)
                    .await?
            }
        };

        match p.read(&char).await {
            Ok(data) => Ok(data),
            Err(_) => {
                self.refresh_peripheral(peri_id).await?;
                let (p, char) = self
                    .get_characteristic(peri_id, service_uuid, char_uuid)
                    .await?;
                p.read(&char)
                    .await
                    .map_err(|e| BleError::new(e.to_string()))
            }
        }
    }

    pub async fn write(
        &mut self,
        peri_id: &str,
        service_uuid: &str,
        char_uuid: &str,
        value: Vec<u8>,
    ) -> Result<(), BleError> {
        let (p, char) = match self
            .get_characteristic(peri_id, service_uuid, char_uuid)
            .await
        {
            Ok(res) => res,
            Err(_) => {
                self.refresh_peripheral(peri_id).await?;
                self.get_characteristic(peri_id, service_uuid, char_uuid)
                    .await?
            }
        };

        match p.write(&char, &value, WriteType::WithResponse).await {
            Ok(_) => Ok(()),
            Err(_) => {
                self.refresh_peripheral(peri_id).await?;
                let (p, char) = self
                    .get_characteristic(peri_id, service_uuid, char_uuid)
                    .await?;
                p.write(&char, &value, WriteType::WithResponse)
                    .await
                    .map_err(|e| BleError::new(e.to_string()))
            }
        }
    }

    pub async fn subscribe(
        &mut self,
        id: &str,
        service_uuid: &str,
        char_uuid: &str,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = NotificationData> + Send>>, BleError>
    {
        let (p, char) = match self.get_characteristic(id, service_uuid, char_uuid).await {
            Ok(characteristic) => characteristic,
            Err(_) => {
                self.refresh_peripheral(id).await?;
                self.get_characteristic(id, service_uuid, char_uuid).await?
            }
        };

        // Register the notification stream before subscribing so the first value is not lost.
        let mut s = p
            .notifications()
            .await
            .map_err(|e| BleError::new(e.to_string()))?;
        if p.subscribe(&char).await.is_err() {
            self.refresh_peripheral(id).await?;
            let (refreshed_p, refreshed_char) =
                self.get_characteristic(id, service_uuid, char_uuid).await?;
            s = refreshed_p
                .notifications()
                .await
                .map_err(|e| BleError::new(e.to_string()))?;
            refreshed_p
                .subscribe(&refreshed_char)
                .await
                .map_err(|e| BleError::new(e.to_string()))?;
        }

        let target_uuid = char.uuid;
        Ok(Box::pin(
            s.filter(move |v| {
                let is_match = v.uuid == target_uuid;
                async move { is_match }
            })
            .map(|v| NotificationData {
                uuid: v.uuid.to_string(),
                value: v.value,
            }),
        ))
    }
    pub async fn is_connected(&mut self, id: &str) -> bool {
        self.connected_peripheral(id).await.is_ok()
    }

    pub async fn list_characteristics(&self, id: &str) -> Result<Vec<UiService>, BleError> {
        let p = self
            .connected_peripherals
            .get(id)
            .ok_or(BleError::new("Device not connected"))?;

        let mut ui_services = Vec::new();
        for service in p.services() {
            let mut ui_chars = Vec::new();
            for char in service.characteristics {
                ui_chars.push(UiCharacteristic {
                    uuid: char.uuid.to_string(),
                    name: characteristic_name(&char.uuid.to_string()).to_string(),
                    service_uuid: service.uuid.to_string(),
                    property: crate::api::prop_flags_to_vec(char.properties),
                });
            }
            ui_services.push(UiService {
                uuid: service.uuid.to_string(),
                name: get_service_name(&service.uuid.to_string()).to_string(),
                characteristic: ui_chars,
            });
        }
        Ok(ui_services)
    }
}

pub fn get_service_name(uuid: &str) -> &str {
    match uuid {
        "00001800-0000-1000-8000-00805f9b34fb" => "Generic Access",
        "00001801-0000-1000-8000-00805f9b34fb" => "Generic Attribute",
        "0000180a-0000-1000-8000-00805f9b34fb" => "Device Information",
        "0000180f-0000-1000-8000-00805f9b34fb" => "Battery Service",
        "00001802-0000-1000-8000-00805f9b34fb" => "Immediate Alert",
        "0000ffe0-0000-1000-8000-00805f9b34fb" => "XSensor Proximity Sensor",
        _ => "Unknown Service",
    }
}

pub async fn is_adapter_available() -> Result<bool, BleError> {
    let manager = Manager::new()
        .await
        .map_err(|e| BleError::new(e.to_string()))?;
    let adpaters = manager
        .adapters()
        .await
        .map_err(|e| BleError::new(e.to_string()))?;
    Ok(!adpaters.is_empty())
}

#[cfg(test)]
mod tests {
    use super::BleManager;
    use crate::ble_constants::{PS_DATA_UUID, SERVICE_UUID};

    #[tokio::test]
    async fn reading_without_a_manual_connection_leaves_the_adapter_unopened() {
        let mut manager = BleManager::new();

        let error = manager
            .read("unconnected", SERVICE_UUID, PS_DATA_UUID)
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "Device not connected");
        assert!(manager.adapter.is_none());
    }
}
