use std::{fs, io, path::PathBuf};
use winreg::enums::*;
use winreg::RegKey;
use native_windows_gui as nwg;

fn main() -> io::Result<()> {
    nwg::init().expect("Failed to init NWG");

    let program_dir = PathBuf::from("C:\\Program Files\\ServeOn8080");
    if !program_dir.exists() {
        fs::create_dir_all(&program_dir)?;
    }

    let source_exe = PathBuf::from("serve_folder.exe");
    let dest_exe = program_dir.join("serve_folder.exe");
    fs::copy(&source_exe, &dest_exe)?;

    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);

    // Background menu
    let (key, _) = hkcr.create_subkey("Directory\\Background\\shell\\ServeOn8080")?;
    key.set_value("", &"Host this folder on port 8080")?;
    let (cmd_key, _) = hkcr.create_subkey("Directory\\Background\\shell\\ServeOn8080\\command")?;
    cmd_key.set_value("", &format!("\"{}\" \"%V\"", dest_exe.display()))?;

    // Folder menu
    let (key2, _) = hkcr.create_subkey("Directory\\shell\\ServeOn8080")?;
    key2.set_value("", &"Host folder on port 8080")?;
    let (cmd_key2, _) = hkcr.create_subkey("Directory\\shell\\ServeOn8080\\command")?;
    cmd_key2.set_value("", &format!("\"{}\" \"%1\"", dest_exe.display()))?;

    show_done_window();

    Ok(())
}

fn show_done_window() {
    let mut window = nwg::Window::default();
    let mut done_btn = nwg::Button::default();

    nwg::Window::builder()
        .size((300, 120))
        .position((600, 400))
        .title("Install Complete")
        .build(&mut window)
        .unwrap();

    nwg::Button::builder()
        .text("Done")
        .parent(&window)
        .size((80, 30))
        .position((110, 60))
        .build(&mut done_btn)
        .unwrap();

    nwg::bind_event_handler(&window.handle, &window.handle, move |evt, _data, _| {
        if let nwg::Event::OnButtonClick = evt {
            nwg::stop_thread_dispatch();
        }
    });

    window.set_visible(true);
    nwg::dispatch_thread_events();
}
