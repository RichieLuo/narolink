#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;
mod engine;
mod file_preview;
mod git;
mod service;
mod settings;
mod syntax;
mod update;
mod web;

fn main() {
    let startup = match update::startup() {
        Ok(Some(startup)) => startup,
        Ok(None) => return,
        Err(problem) => {
            show_error(&format!("{problem:#}"));
            return;
        }
    };
    let testing_launch = startup.testing_launch();
    if let Err(problem) = run(startup) {
        if testing_launch {
            eprintln!("Naro: {problem}");
            std::process::exit(1);
        }
        show_error(&problem);
    }
}

fn show_error(problem: &str) {
    eprintln!("Naro: {problem}");
    let _ = rfd::MessageDialog::new()
        .set_title("Naro")
        .set_description(problem)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

fn run(startup: update::Startup) -> Result<(), String> {
    let options = web::Options::from_environment()?;
    let service = service::Service::default();
    let gateway = web::Gateway::start(service.clone(), options)?;
    if std::env::args().any(|arg| arg == "--web-only") {
        gateway.wait();
        return Ok(());
    }
    desktop::run(service, gateway, startup).map_err(|error| error.to_string())
}
