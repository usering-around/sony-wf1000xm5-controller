use bluer::{Adapter, AdapterEvent, Device, Session};
use eframe::egui::{self, Context, RichText, Slider, Ui};
use futures::{StreamExt, pin_mut};
use sony_wf1000xm5::{
    command::{AncMode, BatteryType, Command, EqualizerPreset},
    message::{BatteryLevel, Codec, Payload},
};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use std::{cell::RefCell, time::Instant};
use tokio::sync::mpsc;

use crate::async_resource::{AsyncResource, ResourceStatus};
use crate::headphone_thread;

const BATTERY_POLL_TIME_SEC: u64 = 60;
struct BtInfo {
    is_powered: bool,
}

struct Equalizer {
    preset: EqualizerPreset,
    clear_bass: i8,
    band_400: i8,
    band_1000: i8,
    band_2500: i8,
    band_6300: i8,
    band_16000: i8,
}

#[derive(Default)]
struct HeadphoneState {
    case_battery: Option<usize>,
    left_ear_battery: Option<usize>,
    right_ear_battery: Option<usize>,
    equalizer: Option<Equalizer>,
    anc_mode: Option<AncMode>,
    ambient_slider: Option<usize>,
    voice_filtering: Option<bool>,
    codec: Option<Codec>,
    last_battery_poll: Option<Instant>,
}

#[derive(Default)]
pub struct App {
    bt_info: AsyncResource<bluer::Result<BtInfo>>,
    bt_devices: Rc<RefCell<HashMap<String, Device>>>,
    bt_devices_task: AsyncResource<bluer::Result<()>>,
    connection_task: AsyncResource<bluer::Result<()>>,
    request_send: Rc<RefCell<Option<mpsc::UnboundedSender<Command>>>>,
    response_recv: Rc<RefCell<Option<mpsc::UnboundedReceiver<Payload>>>>,
    stop_connection_task: Rc<RefCell<Option<mpsc::Sender<()>>>>,
    adapter: Rc<RefCell<Option<Adapter>>>,
    device: String,
    is_connected: bool,
    headphone_state: HeadphoneState,
}

impl App {
    pub fn new() -> Self {
        App::default()
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
                        self.bt_devices_task.set_resource(Ok(()));
                    }
                });
                ui.spinner();
            }

            ResourceStatus::NotInitialized => {
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

    fn start_connection_thread(&self, ctx: &Context) {
        let device = self.bt_devices.borrow().get(&self.device).unwrap().clone();
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (payload_tx, payload_rx) = mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = mpsc::channel(1);
        if let Some(old_stop_tx) = self.stop_connection_task.borrow().as_ref() {
            let _ = old_stop_tx.try_send(());
        }
        *self.stop_connection_task.borrow_mut() = Some(stop_tx);
        *self.request_send.borrow_mut() = Some(command_tx);
        *self.response_recv.borrow_mut() = Some(payload_rx);
        let ctx = ctx.clone();

        self.connection_task.set(async move {
            // we put it in another thread because the UI makes the entire thread sleep.
            // (we could put a timeout in main to prevent it, but I think this option is cleaner)
            tokio::task::spawn_blocking(move || {
                headphone_thread::thread_main(device, payload_tx, command_rx, stop_rx, ctx)
            })
            .await
            .unwrap()
        });
    }

    fn handle_payload(&mut self, payload: Payload) {
        match payload {
            Payload::InitReply => {
                self.is_connected = true;
                let mut tx_borrow = self.request_send.borrow_mut();
                let tx = tx_borrow.as_mut().unwrap();
                // get all information
                tx.send(Command::GetBatteryStatus {
                    battery_type: BatteryType::Headphones,
                })
                .unwrap();
                tx.send(Command::GetBatteryStatus {
                    battery_type: BatteryType::Case,
                })
                .unwrap();
                tx.send(Command::GetEqualizerSettings).unwrap();
                tx.send(Command::GetAncStatus).unwrap();
                tx.send(Command::GetCodec).unwrap();
            }

            Payload::BatteryLevel(battery) => match battery {
                BatteryLevel::Case(battery) => {
                    self.headphone_state.case_battery = Some(battery);
                }

                BatteryLevel::Headphones { left, right } => {
                    self.headphone_state.left_ear_battery = Some(left);
                    self.headphone_state.right_ear_battery = Some(right);
                }
            },

            Payload::Equalizer {
                preset,
                clear_bass,
                band_400,
                band_1000,
                band_2500,
                band_6300,
                band_16000,
            } => {
                self.headphone_state.equalizer = Some(Equalizer {
                    preset,
                    clear_bass,
                    band_400,
                    band_1000,
                    band_2500,
                    band_6300,
                    band_16000,
                });
            }

            Payload::AncStatus {
                mode,
                ambient_sound_voice_filtering,
                ambient_sound_level,
            } => {
                self.headphone_state.anc_mode = Some(mode);
                self.headphone_state.ambient_slider = Some(ambient_sound_level as usize);
                self.headphone_state.voice_filtering = Some(ambient_sound_voice_filtering);
            }

            Payload::Codec { codec } => {
                self.headphone_state.codec = Some(codec);
            }
        }
    }

    // it's written this way to allow functions which do not you the entire self to send a command
    fn send_command(tx: &Rc<RefCell<Option<mpsc::UnboundedSender<Command>>>>, command: Command) {
        if let Some(tx) = tx.borrow().as_ref() {
            tx.send(command).unwrap();
        }
    }

    // written in this way to avoid needing to borrow &mut self
    fn draw_headphones_info(
        state: &mut HeadphoneState,
        ui: &mut Ui,
        request_send: &mut Rc<RefCell<Option<mpsc::UnboundedSender<Command>>>>,
    ) {
        let size = 25.0;
        let last_battey_poll = state.last_battery_poll.unwrap_or(Instant::now());
        if Instant::now() - last_battey_poll > Duration::from_secs(BATTERY_POLL_TIME_SEC) {
            Self::send_command(
                request_send,
                Command::GetBatteryStatus {
                    battery_type: BatteryType::Headphones,
                },
            );
            Self::send_command(
                request_send,
                Command::GetBatteryStatus {
                    battery_type: BatteryType::Case,
                },
            );
        }
        if let Some(left_battery) = state.left_ear_battery
            && let Some(right_battery) = state.right_ear_battery
            && let Some(case_battery) = state.case_battery
        {
            ui.label(
                RichText::from(format!(
                    "🇱 battery: {}, 🇷 battery: {}, case battery: {}",
                    left_battery, right_battery, case_battery
                ))
                .size(size)
                .strong(),
            );
        }
        ui.separator();
        if let Some(codec) = state.codec {
            ui.label(
                RichText::new(format!("Codec: {}", codec.as_str()))
                    .size(size)
                    .strong(),
            );
        }
        ui.separator();
        if let Some(equalizer) = state.equalizer.as_mut() {
            ui.label("Equalizer:");
            ui.menu_button(equalizer.preset.to_string(), |ui| {
                let responses = [
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Off, "Off"),
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Bright, "Bright"),
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Excited, "Excited"),
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Mellow, "Mellow"),
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Relaxed, "Relaxed"),
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Vocal, "Vocal"),
                    ui.selectable_value(
                        &mut equalizer.preset,
                        EqualizerPreset::TrebleBoost,
                        "Treble Boost",
                    ),
                    ui.selectable_value(
                        &mut equalizer.preset,
                        EqualizerPreset::BassBoost,
                        "Bass Boost",
                    ),
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Speech, "Speech"),
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Manual, "Manual"),
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Custom1, "Custom1"),
                    ui.selectable_value(&mut equalizer.preset, EqualizerPreset::Custom2, "Custom2"),
                ];
                if responses.iter().any(|r| r.clicked()) {
                    Self::send_command(
                        request_send,
                        Command::ChangeEqualizerPreset {
                            preset: equalizer.preset,
                        },
                    );
                }
            });

            ui.horizontal(|ui| {
                let responses = vec![
                    ui.add(
                        Slider::new(&mut equalizer.clear_bass, -10..=10)
                            .vertical()
                            .text(RichText::new("clear bass").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_400, -10..=10)
                            .vertical()
                            .text(RichText::new("400").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_1000, -10..=10)
                            .vertical()
                            .text(RichText::new("1000").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_2500, -10..=10)
                            .vertical()
                            .text(RichText::new("2500").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_6300, -10..=10)
                            .vertical()
                            .text(RichText::new("6300").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_16000, -10..=10)
                            .vertical()
                            .text(RichText::new("16000").strong()),
                    ),
                ];
                if responses.iter().any(|r| r.changed()) {
                    Self::send_command(
                        request_send,
                        Command::ChangeEqualizerSetting {
                            bass_level: equalizer.clear_bass,
                            band_400: equalizer.band_400,
                            band_1000: equalizer.band_1000,
                            band_2500: equalizer.band_2500,
                            band_6300: equalizer.band_6300,
                            band_16000: equalizer.band_16000,
                        },
                    );
                }
            });
        }
        ui.separator();
        if let Some(anc_mode) = state.anc_mode.as_mut()
            && let Some(ambient_slider) = state.ambient_slider.as_mut()
            && let Some(voice_filtering) = state.voice_filtering.as_mut()
        {
            ui.label(RichText::new("ANC configuration:").strong().size(size));
            if ui
                .radio_value(anc_mode, AncMode::Off, RichText::new("Off").strong())
                .clicked()
            {
                Self::send_command(
                    request_send,
                    Command::AncSet {
                        dragging_ambient_sound_slider: false,
                        mode: AncMode::Off,
                        ambient_sound_voice_filtering: false,
                        ambient_sound_level: 0,
                    },
                );
            }
            if ui
                .radio_value(
                    anc_mode,
                    AncMode::AmbientSound,
                    RichText::new("Ambient Sounds").strong(),
                )
                .clicked()
            {
                Self::send_command(
                    request_send,
                    Command::AncSet {
                        dragging_ambient_sound_slider: false,
                        mode: AncMode::AmbientSound,
                        ambient_sound_voice_filtering: true,
                        ambient_sound_level: *ambient_slider,
                    },
                );
            }
            if *anc_mode == AncMode::AmbientSound {
                ui.horizontal(|ui| {
                    let mut should_update = false;
                    should_update |= ui.add(Slider::new(ambient_slider, 0..=20)).drag_stopped();
                    should_update |= ui.checkbox(voice_filtering, "voice filtering").clicked();

                    if should_update {
                        Self::send_command(
                            request_send,
                            Command::AncSet {
                                dragging_ambient_sound_slider: false,
                                mode: AncMode::AmbientSound,
                                ambient_sound_voice_filtering: *voice_filtering,
                                ambient_sound_level: *ambient_slider,
                            },
                        );
                    }
                });
            }
            if ui
                .radio_value(
                    anc_mode,
                    AncMode::ActiveNoiseCanceling,
                    RichText::new("Active Noise Canceling").strong(),
                )
                .clicked()
            {
                Self::send_command(
                    request_send,
                    Command::AncSet {
                        dragging_ambient_sound_slider: false,
                        mode: AncMode::ActiveNoiseCanceling,
                        ambient_sound_voice_filtering: true,
                        ambient_sound_level: *ambient_slider,
                    },
                );
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        let rx_clone = self.response_recv.clone();
        if let Some(rx) = rx_clone.borrow_mut().as_mut() {
            while let Ok(payload) = rx.try_recv() {
                self.handle_payload(payload);
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| match self.bt_info.get() {
            ResourceStatus::Ready(bt_info_result) => match bt_info_result.as_ref() {
                Ok(bt_info) => {
                    ui.label(format!("Bluetooth enabled: {}", bt_info.is_powered));
                    if ui.button("refresh").clicked() {
                        self.bt_info.clear();
                    }
                    if !bt_info.is_powered {
                        ui.label("Bluetooth is not on. Turn it on and press refresh.");
                    }
                    else {
                        self.start_device_discovery_task(ctx, ui);
                        for device in self.bt_devices.borrow().keys() {
                            ui.radio_value(&mut self.device, device.clone(), device);
                        }

                        if !self.device.is_empty() {
                            #[allow(clippy::collapsible_if)]
                            if ui.button("connect?").clicked() {
                                self.is_connected = false;
                                self.headphone_state = HeadphoneState::default();
                                self.start_connection_thread(ctx);
                            }
                        }

                        if self.is_connected {
                            ui.label("Connected!");
                            Self::draw_headphones_info(
                                &mut self.headphone_state,
                                ui,
                                &mut self.request_send,
                            );
                        } else {
                            match self.connection_task.get() {
                                ResourceStatus::Ready(result) => {
                                    if let Err(e) = result.as_ref() {
                                        ui.label(format!("Error while connecting: {e}"));
                                    } else {
                                        ui.label("there was a bug; it's likely that the eventloop exited without error for some reason, but the task is still alive.");
                                    }
                                    if ui.button("retry?").clicked() {
                                        self.connection_task.clear();
                                        self.start_connection_thread(ctx);
                                    }
                                }
                                ResourceStatus::Pending => {
                                    ui.label("Trying to connect...");
                                    ui.spinner();
                                }
                                ResourceStatus::NotInitialized => {
                                    ui.label("Not connected");
                                }
                            }
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
                    let adapter = {ui_adapter.borrow().as_ref().unwrap().clone()};

                    Ok(BtInfo {
                        is_powered: adapter.is_powered().await?,
                    })
                });
            }
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // cancel the connection task and all communication to it, since it blocks up the UI on exit
        self.connection_task.cancel();
        self.request_send.take();
        self.response_recv.take();
        self.stop_connection_task.take();
    }
}
