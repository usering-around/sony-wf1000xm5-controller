use crate::async_resource::AsyncResource;
use crate::async_resource::ResourceStatus;
use bluer::{Adapter, AdapterEvent, Device, Session};
use eframe::egui::{self, Context, ScrollArea, Ui};
use futures::StreamExt;
use futures::pin_mut;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

// Might get more info in the future
struct BtInfo {
    is_powered: bool,
}

#[derive(Default)]
pub struct DevicePicker {
    bt_info: AsyncResource<bluer::Result<BtInfo>>,
    bt_devices: Rc<RefCell<HashMap<String, Device>>>,
    bt_devices_task: AsyncResource<anyhow::Result<()>>,
    adapter: Rc<RefCell<Option<Adapter>>>,
    device: String,
    device_addr: String,
    pub last_device_addr: String,
    pub connect_to_the_device_automatically_on_startup: bool,
    found_last_device: bool,
    tried_connecting_to_last_device: bool,
    is_connected: bool,
    wants_connection: Option<Device>,
}

impl DevicePicker {
    pub const LAST_ADDR_KEY: &'static str = "LAST_CONNECTED_DEVICE_ADDRESS";
    pub fn new() -> Self {
        DevicePicker::default()
    }

    fn last_connected_addr(&self) -> Option<&String> {
        if self.last_device_addr.is_empty() {
            None
        } else {
            Some(&self.last_device_addr)
        }
    }

    fn stop_discovery_task(&self) {
        self.bt_devices_task.set_resource(Ok(()));
    }

    fn start_device_discovery_task(&self, ctx: &Context, ui: &mut Ui) {
        match self.bt_devices_task.get() {
            ResourceStatus::Ready(result) => {
                if let Err(e) = result.as_ref() {
                    ui.label(format!("error while discovering devices: {e}"));
                    if ui.button("retry?").clicked() {
                        self.bt_devices_task.clear();
                    }
                } else {
                    ui.label("Search done.");
                    if ui.button("Search again?").clicked() {
                        self.bt_devices_task.clear();
                    }
                }
            }

            ResourceStatus::Pending => {
                ui.horizontal(|ui| {
                    ui.label("Searching devices...");
                    if ui.button("Stop searching?").clicked() {
                        self.stop_discovery_task();
                    }
                });
                ui.spinner();
            }

            ResourceStatus::NotInitialized => {
                {
                    let adapter = self.adapter.borrow().clone().unwrap();
                    // clear the map if we have something in it
                    self.bt_devices.take();
                    let map = self.bt_devices.clone();
                    let ctx = ctx.clone();
                    let timeout = Duration::from_secs(30);
                    self.bt_devices_task.set(async move {
                        let stream = adapter.discover_devices().await?;
                        pin_mut!(stream);
                        let result = tokio::time::timeout(timeout, async move {
                            while let Some(event) = stream.next().await {
                                match event {
                                    AdapterEvent::DeviceAdded(addr) => {
                                        let device = adapter.device(addr)?;
                                        if let Some(name) = device.name().await? {
                                            map.borrow_mut().insert(name, device);
                                            ctx.request_repaint();
                                        }
                                    }

                                    AdapterEvent::DeviceRemoved(addr) => {
                                        let device = adapter.device(addr)?;
                                        if let Some(name) = device.name().await? {
                                            map.borrow_mut().remove(&name);
                                            ctx.request_repaint();
                                        }
                                    }
                                    _ => (),
                                }
                            }
                            Ok(())
                        })
                        .await;
                        match result {
                            Ok(res) => res,
                            Err(_) => Ok(()),
                        }
                    });
                }
            }
        }
    }

    pub fn wants_connection(&mut self) -> Option<Device> {
        self.wants_connection.take()
    }
}

impl eframe::App for DevicePicker {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                match self.bt_info.get() {
                    ResourceStatus::Ready(bt_info_result) => match bt_info_result.as_ref() {
                        Ok(bt_info) => {
                            ui.label(format!("Bluetooth enabled: {}", bt_info.is_powered));
                            if ui.button("refresh").clicked() {
                                self.bt_info.clear();
                            }
                            if !bt_info.is_powered {
                                ui.label("Bluetooth is not on. Turn it on and press refresh.");
                            } else {
                                self.start_device_discovery_task(ctx, ui);
                                for (device, dev) in self.bt_devices.borrow().iter() {
                                    ui.radio_value(&mut self.device, device.clone(), device);
                                    if self.device == *device {
                                        self.device_addr = dev.address().to_string();
                                    }
                                    if self.device.is_empty()
                                        && let Some(addr) = self.last_connected_addr()
                                        && dev.address().to_string() == *addr
                                        && !self.found_last_device
                                    {
                                        self.device = device.clone();
                                        self.found_last_device = true;
                                    }
                                }

                                if !self.device.is_empty() {
                                    #[allow(clippy::collapsible_if)]
                                    if ui.button("connect?").clicked()
                                        || (self.found_last_device
                                            && !self.tried_connecting_to_last_device)
                                    {
                                        // even if we didn't find the last device, if you try to connect to something before we found the device,
                                        // we won't connect.
                                        self.tried_connecting_to_last_device = true;
                                        self.is_connected = false;
                                        self.wants_connection = Some(
                                            self.bt_devices
                                                .borrow()
                                                .get(&self.device)
                                                .unwrap()
                                                .clone(),
                                        );
                                    }

                                    ui.checkbox(
                                        &mut self.connect_to_the_device_automatically_on_startup,
                                        "Connect to this device automatically next time",
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            ui.label(format!("BtInfo: error: {e}"));
                            if ui.button("retry?").clicked() {
                                self.bt_info.clear();
                            }
                        }
                    },

                    ResourceStatus::Pending => {
                        ui.label("Getting BtInfo");
                        ui.spinner();
                    }

                    ResourceStatus::NotInitialized => {
                        let ui_adapter = self.adapter.clone();
                        self.bt_info.set(async move {
                            if ui_adapter.borrow().is_none() {
                                let session = Session::new().await?;
                                let adapter = session.default_adapter().await?;
                                {
                                    *ui_adapter.borrow_mut() = Some(adapter.clone());
                                }
                            }
                            // cloned to not hold it over an await point
                            // i don't think it actually matters in this case, but might as well to remove the clippy warning
                            let adapter = { ui_adapter.borrow().as_ref().unwrap().clone() };

                            Ok(BtInfo {
                                is_powered: adapter.is_powered().await?,
                            })
                        });
                    }
                }
            });
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let device = if self.connect_to_the_device_automatically_on_startup {
            self.device_addr.clone()
        } else {
            String::new()
        };
        storage.set_string(Self::LAST_ADDR_KEY, device);
    }
}
