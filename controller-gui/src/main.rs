use controller_gui::app::App;
#[cfg(not(target_arch = "wasm32"))]
use controller_gui::device_picker::DevicePicker;
#[cfg(not(target_arch = "wasm32"))]
use eframe::egui;

#[cfg(not(target_arch = "wasm32"))]
pub fn main() -> eframe::Result<()> {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _handle = rt.enter();
    std::thread::spawn(move || rt.block_on(std::future::pending::<()>()));

    eframe::run_native(
        "Sony-WF1000XM5 GUI",
        options,
        Box::new(|cc| {
            let mut app = App::default();

            if let Some(storage) = cc.storage
                && let Some(addr) = storage.get_string(DevicePicker::LAST_ADDR_KEY)
                && !addr.is_empty()
            {
                app.picker.last_device_addr = addr;
                app.picker.connect_to_the_device_automatically_on_startup = true;
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::max()).unwrap();

    let web_options = eframe::WebOptions::default();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("No window")
            .document()
            .expect("No document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("Failed to find the_canvas_id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("the_canvas_id was not a HtmlCanvasElement");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|_cc| {
                    let app = App::default();
                    Ok(Box::new(app))
                }),
            )
            .await;

        // Remove the loading text and spinner:
        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => {
                    loading_text.remove();
                }
                Err(e) => {
                    loading_text.set_inner_html(
                        "<p> The app has crashed. See the developer console for details. </p>",
                    );
                    panic!("Failed to start eframe: {e:?}");
                }
            }
        }
    });
}
