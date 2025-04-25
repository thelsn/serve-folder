document.addEventListener('DOMContentLoaded', () => {
    // Elements
    const fileList = document.getElementById('fileList');
    const breadcrumbs = document.getElementById('breadcrumbs');
    const stopServerBtn = document.getElementById('stopServer');
    const confirmModal = document.getElementById('confirmModal');
    const confirmYesBtn = document.getElementById('confirmYes');
    const confirmNoBtn = document.getElementById('confirmNo');
    
    // Current path for navigation
    let currentPath = '';
    
    // Load directory contents
    const loadDirectory = (path = '') => {
        fileList.innerHTML = '<div class="loader">Loading...</div>';
        
        fetch(`/api/list?path=${encodeURIComponent(path)}`)
            .then(response => response.json())
            .then(data => {
                displayFiles(data);
                updateBreadcrumbs(data.current_path);
                currentPath = data.current_path;
            })
            .catch(error => {
                fileList.innerHTML = `<div class="error">Error loading directory: ${error.message}</div>`;
            });
    };
    
    // Display files in the UI
    const displayFiles = (data) => {
        fileList.innerHTML = '';
        
        // Add parent directory link if not at root
        if (data.current_path) {
            const parentPath = data.current_path.split('/').slice(0, -1).join('/');
            const parentItem = document.createElement('div');
            parentItem.className = 'file-item';
            parentItem.innerHTML = `
                <span class="icon folder">📁</span>
                <span class="name">..</span>
                <span class="size">Parent Directory</span>
            `;
            parentItem.addEventListener('click', () => loadDirectory(parentPath));
            fileList.appendChild(parentItem);
        }
        
        // Add all entries
        if (data.entries.length === 0) {
            fileList.innerHTML += '<div class="file-item">No files found</div>';
            return;
        }
        
        data.entries.forEach(entry => {
            const item = document.createElement('div');
            item.className = 'file-item';
            
            if (entry.is_dir) {
                item.innerHTML = `
                    <span class="icon folder">📁</span>
                    <span class="name">${escapeHtml(entry.name)}</span>
                    <span class="size">Folder</span>
                    <div class="actions">
                        <button class="action-btn download" title="Download folder as ZIP">📦</button>
                    </div>
                `;
                
                // Add click event for folder name (navigate)
                const nameEl = item.querySelector('.name');
                nameEl.addEventListener('click', (e) => {
                    e.stopPropagation();
                    loadDirectory(entry.path);
                });
                
                // Add click event for folder download button
                const downloadBtn = item.querySelector('.action-btn.download');
                downloadBtn.addEventListener('click', (e) => {
                    e.stopPropagation(); // Prevent triggering the parent click event
                    downloadFolder(entry.path, entry.name);
                });
                
                // Make folder item clickable for navigation
                item.addEventListener('click', (e) => {
                    if (e.target === item || e.target.classList.contains('icon')) {
                        loadDirectory(entry.path);
                    }
                });
            } else {
                item.innerHTML = `
                    <span class="icon file">📄</span>
                    <span class="name">${escapeHtml(entry.name)}</span>
                    <span class="size">${formatFileSize(entry.size)}</span>
                    <div class="actions">
                        <button class="action-btn download" title="Download this file">⬇️</button>
                    </div>
                `;
                
                // Add click event for the file name (open in new tab)
                const nameEl = item.querySelector('.name');
                nameEl.style.cursor = 'pointer';
                nameEl.addEventListener('click', () => {
                    window.open(`/${entry.path}`, '_blank');
                });
                
                // Add click event for download button
                const downloadBtn = item.querySelector('.action-btn.download');
                downloadBtn.addEventListener('click', (e) => {
                    e.stopPropagation(); // Prevent triggering the parent click event
                    downloadFile(entry.path, entry.name);
                });
            }
            
            fileList.appendChild(item);
        });
    };
    
    // Update breadcrumb navigation
    const updateBreadcrumbs = (path) => {
        breadcrumbs.innerHTML = '<a href="#" data-path="">Root</a>';
        
        if (path) {
            const parts = path.split('/');
            let currentPath = '';
            
            parts.forEach((part, index) => {
                if (part) {
                    currentPath += (currentPath ? '/' : '') + part;
                    breadcrumbs.innerHTML += ` / <a href="#" data-path="${currentPath}">${escapeHtml(part)}</a>`;
                }
            });
        }
        
        // Add click events to breadcrumbs
        breadcrumbs.querySelectorAll('a').forEach(link => {
            link.addEventListener('click', (e) => {
                e.preventDefault();
                loadDirectory(link.getAttribute('data-path'));
            });
        });
    };
    
    // Handle stop server button
    stopServerBtn.addEventListener('click', () => {
        confirmModal.style.display = 'flex';
    });
    
    confirmYesBtn.addEventListener('click', () => {
        fetch('/api/stop', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({ confirm: true })
        })
        .then(response => response.json())
        .then(data => {
            if (data.success) {
                document.body.innerHTML = `
                    <div class="container">
                        <div class="server-stopped">
                            <h1>Server Stopped</h1>
                            <p>The file server has been shut down.</p>
                        </div>
                    </div>
                `;
            } else {
                alert('Failed to stop server: ' + data.message);
            }
        })
        .catch(error => {
            alert('Error stopping server: ' + error.message);
        })
        .finally(() => {
            confirmModal.style.display = 'none';
        });
    });
    
    confirmNoBtn.addEventListener('click', () => {
        confirmModal.style.display = 'none';
    });
    
    // Utility functions
    const formatFileSize = (bytes) => {
        if (bytes === 0) return '0 Bytes';
        const k = 1024;
        const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
    };
    
    const escapeHtml = (unsafe) => {
        return unsafe
            .replace(/&/g, "&amp;")
            .replace(/</g, "&lt;")
            .replace(/>/g, "&gt;")
            .replace(/"/g, "&quot;")
            .replace(/'/g, "&#039;");
    };
    
    // Function to trigger file download
    const downloadFile = (path, filename) => {
        // Create a temporary anchor element
        const anchor = document.createElement('a');
        anchor.href = `/${path}`;
        anchor.download = filename; // This attribute triggers download instead of navigation
        anchor.style.display = 'none';
        document.body.appendChild(anchor);
        
        // Trigger the download
        anchor.click();
        
        // Clean up
        document.body.removeChild(anchor);
    };
    
    // Function to trigger folder download as zip
    const downloadFolder = (path, folderName) => {
        // Show download status in UI with progress bar
        const downloadStatus = document.createElement('div');
        downloadStatus.className = 'download-status';
        downloadStatus.innerHTML = `
            <h4>Downloading ${escapeHtml(folderName)}</h4>
            <p class="current-file">Initializing...</p>
            <div class="progress-container">
                <div class="progress-bar" style="width: 0%"></div>
            </div>
            <p class="progress-text">0%</p>
        `;
        document.body.appendChild(downloadStatus);
        
        // First initialize the ZIP operation to get an operation ID
        console.log(`Initializing ZIP operation for ${path}`);
        fetch(`/api/zip/init?path=${encodeURIComponent(path)}`)
            .then(response => {
                if (!response.ok) throw new Error("Failed to initialize zip operation");
                return response.json();
            })
            .then(data => {
                if (!data.success) {
                    throw new Error("Server reported initialization failure");
                }
                
                const operationId = data.operationId;
                console.log(`Got operation ID: ${operationId}`);
                
                // Start progress polling immediately
                const progressPoller = pollZipProgress(operationId, downloadStatus);
                
                // Then start the actual download
                return fetch(`/api/download/folder?path=${encodeURIComponent(path)}&operation_id=${operationId}`)
                    .then(response => {
                        if (!response.ok) {
                            throw new Error(`HTTP error! Status: ${response.status}`);
                        }
                        return response.blob();
                    })
                    .then(blob => {
                        // Mark polling as complete
                        if (progressPoller.stop) progressPoller.stop();
                        
                        // Show download is complete
                        downloadStatus.querySelector('.current-file').textContent = 'Download complete!';
                        downloadStatus.querySelector('.progress-bar').style.width = '100%';
                        downloadStatus.querySelector('.progress-text').textContent = '100%';
                        
                        // Create download URL
                        const url = window.URL.createObjectURL(blob);
                        
                        // Create and click download link
                        const a = document.createElement('a');
                        a.href = url;
                        a.download = `${folderName}.zip`;
                        a.style.display = 'none';
                        document.body.appendChild(a);
                        a.click();
                        
                        // Clean up
                        window.URL.revokeObjectURL(url);
                        document.body.removeChild(a);
                        
                        // Remove status after delay
                        setTimeout(() => {
                            document.body.removeChild(downloadStatus);
                        }, 3000);
                    });
            })
            .catch(error => {
                console.error('Download error:', error);
                downloadStatus.innerHTML = `<p class="error">Error: ${error.message}</p>`;
                setTimeout(() => {
                    document.body.removeChild(downloadStatus);
                }, 5000);
            });
    };
    
    // Function to poll for zip creation progress
    const pollZipProgress = (operationId, statusElement) => {
        const progressBar = statusElement.querySelector('.progress-bar');
        const progressText = statusElement.querySelector('.progress-text');
        const currentFileElement = statusElement.querySelector('.current-file');
        
        console.log(`Starting progress polling for operation: ${operationId}`);
        let stopPolling = false;
        
        // Function to update the progress bar
        const updateProgress = () => {
            if (stopPolling) return;
            
            fetch(`/api/zip/progress?id=${operationId}`)
                .then(response => response.json())
                .then(data => {
                    console.log('Progress update:', data);
                    
                    // Update progress UI
                    const percentage = Math.min(Math.round(data.percentage), 99);
                    progressBar.style.width = `${percentage}%`;
                    
                    // Format the progress text
                    let statusText = `${percentage}%`;
                    if (data.total_files > 0) {
                        statusText += ` (${data.processed_files}/${data.total_files} files)`;
                    }
                    progressText.textContent = statusText;
                    
                    // Show current file being processed
                    if (data.current_file) {
                        currentFileElement.textContent = data.current_file;
                    }
                    
                    // Continue polling if not complete
                    if (percentage < 99 && !stopPolling) {
                        setTimeout(updateProgress, 300);
                    }
                })
                .catch(error => {
                    console.error('Error checking progress:', error);
                    // Try again unless stopped
                    if (!stopPolling) {
                        setTimeout(updateProgress, 1000);
                    }
                });
        };
        
        // Start polling
        updateProgress();
        
        // Return an object that can be used to stop polling
        return {
            stop: () => { stopPolling = true; }
        };
    };
    
    // Initialize the file browser
    loadDirectory();
});
