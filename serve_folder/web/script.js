document.addEventListener('DOMContentLoaded', () => {
    let currentPath = '';

    // Register Service Worker for PWA
    if ('serviceWorker' in navigator) {
        navigator.serviceWorker.register('/service-worker.js')
            .then(registration => console.log('Service Worker registered'))
            .catch(error => console.log('Service Worker registration failed:', error));
    }

    // Load directory listing
    const loadDirectory = (path = '') => {
        currentPath = path;
        const fileList = document.getElementById('fileList');
        fileList.innerHTML = '<div class="loader">Loading...</div>';

        fetch(`/api/list?path=${encodeURIComponent(path)}`)
            .then(response => response.json())
            .then(data => {
                renderBreadcrumb(data.current_path);
                renderFileList(data.entries);
            })
            .catch(error => {
                console.error('Error loading directory:', error);
                fileList.innerHTML = '<p class="error">Failed to load directory</p>';
            });
    };

    // Render breadcrumb navigation
    const renderBreadcrumb = (path) => {
        const breadcrumb = document.getElementById('breadcrumb');
        const parts = path ? path.split('/').filter(p => p) : [];
        
        let html = '<a href="#" data-path="">🏠 Home</a>';
        let currentPath = '';
        
        parts.forEach(part => {
            currentPath += (currentPath ? '/' : '') + part;
            html += ` / <a href="#" data-path="${currentPath}">${part}</a>`;
        });
        
        breadcrumb.innerHTML = html;
        
        // Add click handlers
        breadcrumb.querySelectorAll('a').forEach(link => {
            link.addEventListener('click', (e) => {
                e.preventDefault();
                loadDirectory(link.dataset.path);
            });
        });
    };

    // Render file list
    const renderFileList = (entries) => {
        const fileList = document.getElementById('fileList');
        
        if (entries.length === 0) {
            fileList.innerHTML = '<p class="empty">This folder is empty</p>';
            return;
        }
        
        let html = '<table class="file-table"><thead><tr><th>Name</th><th>Size</th><th>Actions</th></tr></thead><tbody>';
        
        entries.forEach(entry => {
            const icon = entry.is_dir ? '📁' : '📄';
            const size = entry.is_dir ? '-' : formatFileSize(entry.size);
            
            html += `<tr>
                <td class="file-name">
                    ${icon} 
                    ${entry.is_dir 
                        ? `<a href="#" class="dir-link" data-path="${entry.path}">${entry.name}</a>`
                        : `<span>${entry.name}</span>`
                    }
                </td>
                <td class="file-size">${size}</td>
                <td class="file-actions">
                    ${entry.is_dir 
                        ? `<button class="btn btn-sm" onclick="downloadFolder('${entry.path}')">📦 Download ZIP</button>`
                        : ''
                    }
                </td>
            </tr>`;
        });
        
        html += '</tbody></table>';
        fileList.innerHTML = html;
        
        // Add click handlers for directories
        fileList.querySelectorAll('.dir-link').forEach(link => {
            link.addEventListener('click', (e) => {
                e.preventDefault();
                loadDirectory(link.dataset.path);
            });
        });
    };

    // Format file size
    const formatFileSize = (bytes) => {
        if (bytes === 0) return '0 B';
        const k = 1024;
        const sizes = ['B', 'KB', 'MB', 'GB'];
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i];
    };

    // Download folder as ZIP
    window.downloadFolder = async (path) => {
        const folderName = path.split('/').filter(Boolean).pop() || 'folder';
        let progressInterval = null;

        try {
            const initResponse = await fetch(`/api/zip/init?path=${encodeURIComponent(path)}`);
            const initData = await initResponse.json();

            if (!initData.success) {
                alert('Failed to initialize ZIP operation');
                return;
            }

            const operationId = initData.operationId;

            const statusDiv = document.createElement('div');
            statusDiv.className = 'download-status';
            statusDiv.innerHTML = `
                <p class="current-file">Preparing download...</p>
                <div class="progress-container">
                    <div class="progress-bar" style="width: 0%"></div>
                </div>
                <p class="progress-text">0%</p>
            `;
            document.body.appendChild(statusDiv);

            const progressBar = statusDiv.querySelector('.progress-bar');
            const progressText = statusDiv.querySelector('.progress-text');
            const currentFile = statusDiv.querySelector('.current-file');

            progressInterval = setInterval(async () => {
                try {
                    const progressResponse = await fetch(`/api/zip/progress?id=${encodeURIComponent(operationId)}`);
                    const progress = await progressResponse.json();

                    progressBar.style.width = `${progress.percentage}%`;
                    progressText.textContent = progress.total_files > 0
                        ? `${Math.round(progress.percentage)}% (${progress.processed_files}/${progress.total_files})`
                        : `${Math.round(progress.percentage)}%`;
                    currentFile.textContent = progress.current_file || 'Preparing download...';
                } catch (error) {
                    console.error('Progress polling error:', error);
                }
            }, 250);

            const response = await fetch(`/api/download/folder?path=${encodeURIComponent(path)}&operation_id=${encodeURIComponent(operationId)}`);

            clearInterval(progressInterval);
            progressInterval = null;

            if (!response.ok) {
                throw new Error('Download failed');
            }

            currentFile.textContent = 'Saving ZIP file...';
            progressBar.style.width = '100%';
            progressText.textContent = '100%';

            const blob = await response.blob();
            const url = URL.createObjectURL(blob);
            const link = document.createElement('a');
            link.href = url;
            link.download = `${folderName}.zip`;
            document.body.appendChild(link);
            link.click();
            document.body.removeChild(link);
            URL.revokeObjectURL(url);

            currentFile.textContent = 'Download complete';
            setTimeout(() => {
                statusDiv.remove();
            }, 1500);
        } catch (error) {
            if (progressInterval) {
                clearInterval(progressInterval);
            }
            console.error('Download error:', error);
            alert('Failed to download folder');
        }
    };

    // Upload functionality
    const uploadBtn = document.getElementById('uploadBtn');
    const uploadModal = document.getElementById('uploadModal');
    const closeModal = document.querySelector('.close');
    const uploadArea = document.getElementById('uploadArea');
    const fileInput = document.getElementById('fileInput');
    const uploadProgress = document.getElementById('uploadProgress');

    uploadBtn.addEventListener('click', () => {
        uploadModal.style.display = 'block';
    });

    closeModal.addEventListener('click', () => {
        uploadModal.style.display = 'none';
    });

    window.addEventListener('click', (event) => {
        if (event.target === uploadModal) {
            uploadModal.style.display = 'none';
        }
    });

    uploadArea.addEventListener('click', () => {
        fileInput.click();
    });

    fileInput.addEventListener('change', (e) => {
        if (e.target.files.length > 0) {
            uploadFiles(Array.from(e.target.files));
        }
    });

    // Drag and drop
    uploadArea.addEventListener('dragover', (e) => {
        e.preventDefault();
        uploadArea.classList.add('dragover');
    });

    uploadArea.addEventListener('dragleave', () => {
        uploadArea.classList.remove('dragover');
    });

    uploadArea.addEventListener('drop', (e) => {
        e.preventDefault();
        uploadArea.classList.remove('dragover');
        
        const files = Array.from(e.dataTransfer.files);
        if (files.length > 0) {
            uploadFiles(files);
        }
    });

    // Upload files function
    const uploadFiles = async (files) => {
        const formData = new FormData();
        
        files.forEach(file => {
            formData.append('file', file);
        });

        uploadArea.style.display = 'none';
        uploadProgress.style.display = 'block';
        
        const progressBar = uploadProgress.querySelector('.progress-bar');
        const progressText = uploadProgress.querySelector('.progress-text');
        const uploadStatus = uploadProgress.querySelector('.upload-status');

        try {
            const response = await fetch(`/api/upload?path=${encodeURIComponent(currentPath)}`, {
                method: 'POST',
                body: formData
            });

            if (!response.ok) {
                throw new Error('Upload failed');
            }
 
            const result = await response.json();
            
            progressBar.style.width = '100%';
            progressText.textContent = '100%';
            uploadStatus.textContent = `✅ Successfully uploaded ${result.count} file(s)`;
            
            setTimeout(() => {
                uploadModal.style.display = 'none';
                uploadArea.style.display = 'block';
                uploadProgress.style.display = 'none';
                progressBar.style.width = '0%';
                progressText.textContent = '0%';
                fileInput.value = '';
                loadDirectory(currentPath);
            }, 1500);
            
        } catch (error) {
            console.error('Upload error:', error);
            uploadStatus.textContent = '❌ Upload failed: ' + error.message;
            
            setTimeout(() => {
                uploadArea.style.display = 'block';
                uploadProgress.style.display = 'none';
            }, 2000);
        }
    };

    // Stop server button
    const stopBtn = document.getElementById('stopBtn');
    stopBtn.addEventListener('click', async () => {
        if (confirm('Are you sure you want to stop the server?')) {
            try {
                await fetch('/api/stop', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ confirm: true })
                });
                alert('Server is shutting down...');
            } catch (error) {
                console.error('Error stopping server:', error);
            }
        }
    });

    // Load initial directory
    loadDirectory();
});
