use controller_gui::app::App;
use eframe::{EframePumpStatus, UserEvent, egui};
use std::{io, os::fd::AsRawFd};
use tokio::task::LocalSet;
use winit::event_loop::{ControlFlow, EventLoop};

pub fn main() -> io::Result<()> {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };

    let mut eventloop = EventLoop::<UserEvent>::with_user_event().build().unwrap();
    eventloop.set_control_flow(ControlFlow::Poll);

    let mut winit_app = eframe::create_native(
        "External Eventloop Application",
        options,
        Box::new(|cc| {
            let mut app = App::default();
            if let Some(storage) = cc.storage
                && let Some(addr) = storage.get_string(App::LAST_ADDR_KEY)
                && !addr.is_empty()
            {
                app.last_device_addr = addr;
                app.connect_to_the_device_automatically_on_startup = true;
            }
            Ok(Box::new(app))
        }),
        &eventloop,
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = LocalSet::new();
    local.block_on(&rt, async {
        let eventloop_fd = tokio::io::unix::AsyncFd::new(eventloop.as_raw_fd())?;
        let mut control_flow = ControlFlow::Poll;

        loop {
            let mut guard = match control_flow {
                ControlFlow::Poll => None,
                ControlFlow::Wait => Some(eventloop_fd.readable().await?),
                ControlFlow::WaitUntil(deadline) => {
                    tokio::time::timeout_at(deadline.into(), eventloop_fd.readable())
                        .await
                        .ok()
                        .transpose()?
                }
            };

            match winit_app.pump_eframe_app(&mut eventloop, None) {
                EframePumpStatus::Continue(next) => control_flow = next,
                EframePumpStatus::Exit(_code) => {
                    break;
                }
            }

            if let Some(mut guard) = guard.take() {
                guard.clear_ready();
            }
        }

        Ok::<_, io::Error>(())
    })
}
