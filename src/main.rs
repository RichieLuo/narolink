#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;
mod engine;
mod service;
mod settings;
mod web;

fn main() {
    if let Err(problem) = run() {
        eprintln!("Naro: {problem}");
        let _ = rfd::MessageDialog::new()
            .set_title("Naro")
            .set_description(&problem)
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}

fn run() -> Result<(), String> {
    let options = web::Options::from_environment()?;
    let service = service::Service::default();
    let gateway = web::Gateway::start(service.clone(), options)?;
    if std::env::args().any(|arg| arg == "--web-only") {
        gateway.wait();
        return Ok(());
    }
    desktop::run(service, gateway).map_err(|error| error.to_string())
}
