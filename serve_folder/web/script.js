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
                `;
                item.addEventListener('click', () => loadDirectory(entry.path));
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
    
    // Initialize the file browser
    loadDirectory();
});
