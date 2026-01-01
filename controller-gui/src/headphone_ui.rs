use crate::async_resource::AsyncResource;
use eframe::egui::{self, RichText, Slider, Ui};
#[cfg(target_arch = "wasm32")]
use futures::StreamExt;
use sony_wf1000xm5::{
    command::{AncMode, BatteryType, Command, EqualizerPreset},
    payload::{BatteryLevel, Codec, Payload},
};
use tokio::sync::mpsc;

#[derive(PartialEq, Eq)]
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
    voice_passthrough: Option<bool>,
    codec: Option<Codec>,
    sound_pressure_db: Option<usize>,
    sound_pressure_poll_task: AsyncResource<()>,
}

pub struct HeadphoneUi {
    request_send: mpsc::UnboundedSender<Command>,
    payload_recv: mpsc::UnboundedReceiver<Payload>,
    stop_connection: mpsc::Sender<()>,
    headphone_state: HeadphoneState,
    is_connected: bool,
}

impl HeadphoneUi {
    pub fn new(
        request_send: mpsc::UnboundedSender<Command>,
        payload_recv: mpsc::UnboundedReceiver<Payload>,
        stop_connection: mpsc::Sender<()>,
    ) -> Self {
        Self {
            request_send,
            payload_recv,
            stop_connection,
            headphone_state: HeadphoneState::default(),
            is_connected: false,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected
    }
    fn handle_payload(&mut self, payload: Payload) {
        match payload {
            Payload::InitReply => {
                self.is_connected = true;
                // get all information
                self.request_send
                    .send(Command::GetBatteryStatus {
                        battery_type: BatteryType::Headphones,
                    })
                    .unwrap();
                self.request_send
                    .send(Command::GetBatteryStatus {
                        battery_type: BatteryType::Case,
                    })
                    .unwrap();
                self.request_send
                    .send(Command::GetEqualizerSettings)
                    .unwrap();
                self.request_send.send(Command::GetAncStatus).unwrap();
                self.request_send.send(Command::GetCodec).unwrap();
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
                ambient_sound_voice_passthrough,
                ambient_sound_level,
            } => {
                self.headphone_state.anc_mode = Some(mode);
                self.headphone_state.ambient_slider = Some(ambient_sound_level as usize);
                self.headphone_state.voice_passthrough = Some(ambient_sound_voice_passthrough);
            }

            Payload::Codec { codec } => {
                self.headphone_state.codec = Some(codec);
            }

            Payload::SoundPressureMeasureReply { is_on } => {
                if is_on {
                    self.request_send.send(Command::GetSoundPressure).unwrap();
                    let request_send = self.request_send.clone();
                    // we create the polling task in another thread since the GUI thread sleeps when there is no user interaction
                    #[cfg(not(target_arch = "wasm32"))]
                    self.headphone_state
                        .sound_pressure_poll_task
                        .set(async move {
                            let (stop_tx, mut stop_rx) = mpsc::channel(1);
                            let _ = tokio::task::spawn_blocking(move || {
                                tokio::runtime::Builder::new_current_thread()
                                    .enable_time()
                                    .build()
                                    .unwrap()
                                    .block_on(async move {
                                        loop {
                                            use std::time::Duration;

                                            tokio::select! {
                                                _ = stop_rx.recv() => {
                                                    break;
                                                }

                                                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                                                    if request_send.send(Command::GetSoundPressure).is_err()
                                                    {
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        log::debug!("sound pressure task dead");
                                    });
                            })
                            .await;
                            let _ = stop_tx.send(()).await;
                        });

                    #[cfg(target_arch = "wasm32")]
                    self.headphone_state
                        .sound_pressure_poll_task
                        .set(async move {
                            let mut interval = gloo_timers::future::IntervalStream::new(1000);
                            while let Some(_) = interval.next().await {
                                if request_send.send(Command::GetSoundPressure).is_err() {
                                    break;
                                }
                            }
                        });
                } else {
                    self.headphone_state.sound_pressure_db = None;
                    self.headphone_state.sound_pressure_poll_task.cancel();
                }
            }

            Payload::SoundPressure { db } => {
                self.headphone_state.sound_pressure_db = Some(db);
            }
        }
    }

    fn draw_headphones_info(&mut self, ui: &mut Ui) {
        let size = 25.0;

        if ui.button("disconnect?").clicked() {
            self.stop_connection.try_send(()).unwrap();
        }
        if let Some(left_battery) = self.headphone_state.left_ear_battery
            && let Some(right_battery) = self.headphone_state.right_ear_battery
            && let Some(case_battery) = self.headphone_state.case_battery
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
        if let Some(codec) = self.headphone_state.codec {
            ui.label(
                RichText::new(format!("Codec: {}", codec.as_str()))
                    .size(size)
                    .strong(),
            );
        }
        ui.separator();
        if let Some(sound_pressure) = self.headphone_state.sound_pressure_db {
            ui.label(
                RichText::new(format!("sound pressure: {sound_pressure} dB"))
                    .strong()
                    .size(size),
            );
            if ui.button("stop?").clicked() {
                self.request_send
                    .send(Command::SoundPressureMeasure { on: false })
                    .unwrap();
            }
        } else if ui.button("Start sound pressure measure?").clicked() {
            self.request_send
                .send(Command::SoundPressureMeasure { on: true })
                .unwrap();
        }

        ui.separator();
        if let Some(equalizer) = self.headphone_state.equalizer.as_mut() {
            ui.label(RichText::new("Equalizer").strong().size(size));

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
                    self.request_send
                        .send(Command::ChangeEqualizerPreset {
                            preset: equalizer.preset,
                        })
                        .unwrap();
                }
            });

            ui.horizontal(|ui| {
                let responses = [
                    ui.add(
                        Slider::new(&mut equalizer.clear_bass, -10..=10)
                            .vertical()
                            .text(RichText::new("clear bass").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_400, -10..=10)
                            .vertical()
                            .text(RichText::new("400 Hz").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_1000, -10..=10)
                            .vertical()
                            .text(RichText::new("1000 Hz").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_2500, -10..=10)
                            .vertical()
                            .text(RichText::new("2500 Hz").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_6300, -10..=10)
                            .vertical()
                            .text(RichText::new("6300 Hz").strong()),
                    ),
                    ui.add(
                        Slider::new(&mut equalizer.band_16000, -10..=10)
                            .vertical()
                            .text(RichText::new("16000 Hz").strong()),
                    ),
                ];
                if responses.iter().any(|r| r.changed()) {
                    let preset = if matches!(
                        equalizer.preset,
                        EqualizerPreset::Manual
                            | EqualizerPreset::Custom1
                            | EqualizerPreset::Custom2
                    ) {
                        equalizer.preset
                    } else {
                        // we shouldn't (can't?) change non-custom/manual presets
                        EqualizerPreset::Manual
                    };
                    self.request_send
                        .send(Command::ChangeEqualizerSetting {
                            preset,
                            bass_level: equalizer.clear_bass,
                            band_400: equalizer.band_400,
                            band_1000: equalizer.band_1000,
                            band_2500: equalizer.band_2500,
                            band_6300: equalizer.band_6300,
                            band_16000: equalizer.band_16000,
                        })
                        .unwrap();
                }
            });
        }
        ui.separator();
        if let Some(anc_mode) = self.headphone_state.anc_mode.as_mut()
            && let Some(ambient_slider) = self.headphone_state.ambient_slider.as_mut()
            && let Some(voice_passthrough) = self.headphone_state.voice_passthrough.as_mut()
        {
            ui.label(RichText::new("ANC configuration:").strong().size(size));
            if ui
                .radio_value(anc_mode, AncMode::Off, RichText::new("Off").strong())
                .clicked()
            {
                self.request_send
                    .send(Command::AncSet {
                        dragging_ambient_sound_slider: false,
                        mode: AncMode::Off,
                        ambient_sound_voice_passthrough: false,
                        ambient_sound_level: 0,
                    })
                    .unwrap();
            }
            if ui
                .radio_value(
                    anc_mode,
                    AncMode::AmbientSound,
                    RichText::new("Ambient Sounds").strong(),
                )
                .clicked()
            {
                self.request_send
                    .send(Command::AncSet {
                        dragging_ambient_sound_slider: false,
                        mode: AncMode::AmbientSound,
                        ambient_sound_voice_passthrough: true,
                        ambient_sound_level: *ambient_slider,
                    })
                    .unwrap();
            }
            if *anc_mode == AncMode::AmbientSound {
                ui.horizontal(|ui| {
                    let mut should_update = false;
                    should_update |= ui.add(Slider::new(ambient_slider, 0..=20)).drag_stopped();
                    should_update |= ui
                        .checkbox(voice_passthrough, "voice passthrough")
                        .clicked();

                    if should_update {
                        self.request_send
                            .send(Command::AncSet {
                                dragging_ambient_sound_slider: false,
                                mode: AncMode::AmbientSound,
                                ambient_sound_voice_passthrough: *voice_passthrough,
                                ambient_sound_level: *ambient_slider,
                            })
                            .unwrap();
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
                self.request_send
                    .send(Command::AncSet {
                        dragging_ambient_sound_slider: false,
                        mode: AncMode::ActiveNoiseCanceling,
                        ambient_sound_voice_passthrough: true,
                        ambient_sound_level: *ambient_slider,
                    })
                    .unwrap();
            }
        }
    }
    pub fn poll_events(&mut self) {
        while let Ok(payload) = self.payload_recv.try_recv() {
            self.handle_payload(payload);
        }
    }
}

impl eframe::App for HeadphoneUi {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_headphones_info(ui);
        });
    }
}
