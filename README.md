# ServeOn8080

A simple, easy-to-use file server that lets you quickly share files from any folder on your Windows computer.

## Overview

ServeOn8080 consists of two components:

1. **serve_folder** - A lightweight HTTP server that serves files from a specified directory
2. **serve_installer** - A Windows installer that adds convenient right-click menu options

Once installed, you can right-click on any folder in File Explorer and select "Host folder on port 8080" to instantly share those files on your local network.

## Installation

### Prerequisites

- Windows 10/11
- Administrator privileges (required for installer)

### Steps

1. Build both components (or download pre-built binaries)
   ```
   cd serve_folder
   cargo build --release
   
   cd ../serve_installer
   cargo build --release
   ```

2. Copy the `serve_folder.exe` to the same directory as `serve_installer.exe`

3. Run `serve_installer.exe` with administrator privileges
   - This installs the application to `C:\Program Files\ServeOn8080\`
   - Adds right-click context menu options to Windows Explorer

## Usage

### Starting a server

1. Navigate to any folder in Windows Explorer
2. Right-click on the folder (or in an empty space within the folder)
3. Select "Host folder on port 8080" or "Host this folder on port 8080"
4. A command prompt window will open showing the server is running
5. Open your browser and go to [http://127.0.0.1:8080](http://127.0.0.1:8080)

### Using the web interface

- Browse folders by clicking on directory names
- Use the breadcrumb navigation to go back up the directory tree
- Click on file names to open them in a new browser tab
- Use the download button (⬇️) to download files


