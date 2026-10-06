# ServeOn8080

A simple, easy-to-use file server that lets you quickly share files from any folder on your Windows computer.

## Overview

ServeOn8080 consists of two components:

1. **serve_folder** - A lightweight HTTP server that serves files from a specified directory
2. **installer** - Batch scripts that install it and add convenient right-click menu options

Once installed, you can right-click on any folder in File Explorer and select "Host folder on port 8080" to instantly share those files on your local network.

## Installation

### Prerequisites

- Windows 10/11
- Administrator privileges (the installer asks for them)

### Steps

1. Download `ServeOn8080-<version>-windows-x64.zip` from the [latest release](https://github.com/thelsn/serve-folder/releases/latest)
2. Extract the whole zip
3. Double-click `install.bat` and accept the administrator prompt
   - This installs the application to `C:\Program Files\ServeOn8080\`
   - Adds right-click context menu options to Windows Explorer

Running `install.bat` again updates an existing install. To remove ServeOn8080, run `uninstall.bat`.

## Usage

### Starting a server

1. Navigate to any folder in Windows Explorer
2. Right-click on the folder (or in an empty space within the folder)
3. Select "Host folder on port 8080" or "Host this folder on port 8080" (on Windows 11, under "Show more options")
4. A command prompt window will open showing the server is running
5. Open your browser and go to [http://127.0.0.1:8080](http://127.0.0.1:8080), or `http://<this PC's IP>:8080` from other devices on your network

You can also run it directly: `serve_folder.exe <folder>`

### Using the web interface

- Browse folders by clicking on directory names
- Use the breadcrumb navigation to go back up the directory tree
- Click on file names to open them in a new browser tab
- Use the download button (⬇️) to download files
- Use **📦 Download ZIP** to download a whole folder. The zip is streamed while it's built, so large folders start downloading straight away
- Upload with **📤 Upload Files** (into the current folder) or a folder's **📤 Upload** button. You can pick files or a whole folder, or drag them anywhere onto the page. Existing files are never overwritten: a file with the same name is saved as `name (1).ext`

## Building from source

```
cd serve_folder
cargo build --release
```

The program is written to `serve_folder/target/release/serve_folder.exe`. To install your own build, put it next to `installer/install.bat` and run the installer.
