use crate::async_resource::ResourceStatus;
#[cfg(not(target_arch = "wasm32"))]
use crate::device_picker::DevicePicker;
use crate::headphone_thread;
use crate::{async_resource::AsyncResource, headphone_ui::HeadphoneUi};
#[cfg(not(target_arch = "wasm32"))]
use bluer::Device;
use eframe::egui;
use tokio::sync::mpsc;
#[cfg(target_arch = "wasm32")]
use web_sys::SerialPort;

#[derive(Default)]
pub struct App {
    #[cfg(not(target_arch = "wasm32"))]
    pub picker: DevicePicker,
    #[cfg(not(target_arch = "wasm32"))]
    current_connection: Option<Device>,
    #[cfg(target_arch = "wasm32")]
    current_connection: Option<SerialPort>,
    #[cfg(target_arch = "wasm32")]
    picker: AsyncResource<anyhow::Result<SerialPort>>,
    connection_task: AsyncResource<anyhow::Result<()>>,
    headphone_ui: Option<HeadphoneUi>,
}

impl App {
    #[cfg(target_arch = "wasm32")]
    fn pick_device_web(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| match self.picker.get() {
            ResourceStatus::Ready(result) => {
                if let Err(e) = result.as_ref() {
                    ui.label(format!("Error while requesting permissions: {e}"));
                } else {
                    self.current_connection = Some(result.as_ref().unwrap().clone());
                }
            }
            ResourceStatus::Pending => {
                ui.label("Pick the headphones from the popup");
                ui.spinner();
            }
            ResourceStatus::NotInitialized => {
                if ui.button("Allow connection to WF-1000XM5").clicked() {
                    use eframe::wasm_bindgen::JsValue;
                    use wasm_bindgen_futures::JsFuture;
                    use web_sys::{
                        SerialPortRequestOptions,
                        js_sys::{Array, Reflect},
                    };

                    let navigator = web_sys::window().expect("no window?").navigator();
                    let serial = navigator.serial();
                    let options = SerialPortRequestOptions::new();
                    let filters = web_sys::js_sys::JSON::parse(
                        r#"
                        [
                        {
                            "bluetoothServiceClassId":  ["956c7b26-d49a-4ba8-b03f-b17d393cb6e2"] 
                        }
                        ]
                    "#,
                    )
                    .unwrap();
                    options.set_filters(&filters);
                    let uuid_array = Array::new();
                    uuid_array.push(&JsValue::from_str("956c7b26-d49a-4ba8-b03f-b17d393cb6e2"));
                    Reflect::set(
                        &options,
                        &JsValue::from_str("allowedBluetoothServiceClassIds"),
                        &uuid_array,
                    )
                    .unwrap();
                    let future = JsFuture::from(serial.request_port_with_options(&options));
                    self.picker.set(async move {
                        use eframe::wasm_bindgen::JsCast;
                        use web_sys::SerialPort;

                        let port: SerialPort = future.await.unwrap().dyn_into().unwrap();
                        log::error!("got serial port");
                        Ok(port)
                    });
                }
            }
        });
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.current_connection.is_none() {
            #[cfg(target_os = "linux")]
            {
                self.picker.update(ctx, frame);
                self.current_connection = self.picker.wants_connection();
            }
            #[cfg(target_arch = "wasm32")]
            {
                self.pick_device_web(ctx, frame);
            }
        } else {
            let mut should_reset_connection = false;
            match self.connection_task.get() {
                ResourceStatus::Ready(result) => {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if let Err(e) = result.as_ref() {
                            ui.label(format!("Got an error: {e}"));
                        } else {
                            // if it dies with Ok(()) it means the user disconnected by themselves
                            should_reset_connection = true;
                        }
                        if ui.button("retry?").clicked() {
                            self.connection_task.clear();
                        }
                        if ui.button("go back to device picker").clicked() {
                            should_reset_connection = true;
                        }
                    });
                }

                ResourceStatus::Pending => {
                    let headphone_ui = self.headphone_ui.as_mut().unwrap();
                    if headphone_ui.is_connected() {
                        headphone_ui.update(ctx, frame);
                    } else {
                        headphone_ui.poll_events();
                        egui::CentralPanel::default().show(ctx, |ui| {
                            ui.label("Connecting...");
                            if ui.button("stop?").clicked() {
                                should_reset_connection = true;
                            }
                            ui.spinner();
                        });
                    }
                }
                ResourceStatus::NotInitialized => {
                    let (command_tx, command_rx) = mpsc::unbounded_channel();
                    let (payload_tx, payload_rx) = mpsc::unbounded_channel();
                    let (stop_tx, stop_rx) = mpsc::channel(1);
                    #[cfg(not(target_arch = "wasm32"))]
                    let device = self.current_connection.as_ref().unwrap().clone();
                    #[cfg(target_arch = "wasm32")]
                    let port = self.current_connection.as_ref().unwrap().clone();
                    let ctx = ctx.clone();
                    #[cfg(not(target_arch = "wasm32"))]
                    self.connection_task.set(async move {
                        tokio::task::spawn_blocking(move || {
                            headphone_thread::thread_main(
                                device, payload_tx, command_rx, stop_rx, ctx,
                            )
                        })
                        .await?
                    });
                    #[cfg(target_arch = "wasm32")]
                    self.connection_task.set(async move {
                        headphone_thread::thread_main(port, payload_tx, command_rx, stop_rx, ctx)
                            .await
                    });
                    self.headphone_ui = Some(HeadphoneUi::new(command_tx, payload_rx, stop_tx));
                }
            }
            if should_reset_connection {
                self.connection_task.clear();
                self.current_connection = None;

                #[cfg(target_arch = "wasm32")]
                self.picker.clear();
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // cancel the connection task and all communication to it, since it blocks up the UI on exit
        self.connection_task.cancel();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.picker.save(storage);
    }
}
