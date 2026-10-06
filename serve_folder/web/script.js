document.addEventListener('DOMContentLoaded', () => {
    let currentPath = '';

    const fileList = document.getElementById('fileList');
    const breadcrumb = document.getElementById('breadcrumbs');

    // Register Service Worker for PWA
    if ('serviceWorker' in navigator) {
        navigator.serviceWorker.register('/webui/service-worker.js')
            .then(registration => console.log('Service Worker registered'))
            .catch(error => console.log('Service Worker registration failed:', error));
    }

    // File and folder names come from disk and can contain any character
    const escapeHtml = (text) => String(text)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;');

    // URL of a file served by the static file route
    const fileUrl = (path) => '/' + path.split('/').map(encodeURIComponent).join('/');

    // Load directory listing
    const loadDirectory = (path = '') => {
        fileList.innerHTML = '<div class="loader">Loading...</div>';

        fetch(`/api/list?path=${encodeURIComponent(path)}`)
            .then(response => {
                if (!response.ok) {
                    throw new Error(`Server responded ${response.status}`);
                }
                return response.json();
            })
            .then(data => {
                currentPath = data.current_path;
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
        const parts = path ? path.split('/').filter(p => p) : [];

        let html = '<a href="#" data-path="">🏠 Home</a>';
        let crumbPath = '';

        parts.forEach(part => {
            crumbPath += (crumbPath ? '/' : '') + part;
            html += ` / <a href="#" data-path="${escapeHtml(crumbPath)}">${escapeHtml(part)}</a>`;
        });

        breadcrumb.innerHTML = html;
    };

    breadcrumb.addEventListener('click', (e) => {
        const link = e.target.closest('a[data-path]');
        if (link) {
            e.preventDefault();
            loadDirectory(link.dataset.path);
        }
    });

    // Render file list
    const renderFileList = (entries) => {
        if (entries.length === 0) {
            fileList.innerHTML = '<p class="empty">This folder is empty</p>';
            return;
        }

        let html = '<table class="file-table"><thead><tr><th>Name</th><th>Size</th><th>Actions</th></tr></thead><tbody>';

        entries.forEach(entry => {
            const name = escapeHtml(entry.name);
            const path = escapeHtml(entry.path);

            if (entry.is_dir) {
                html += `<tr>
                    <td class="file-name">📁 <a href="#" class="dir-link" data-path="${path}">${name}</a></td>
                    <td class="file-size">-</td>
                    <td class="file-actions">
                        <button class="btn btn-sm upload-here-btn" data-path="${path}" title="Upload files into this folder">📤 Upload</button>
                        <button class="btn btn-sm zip-btn" data-path="${path}">📦 Download ZIP</button>
                    </td>
                </tr>`;
            } else {
                const url = escapeHtml(fileUrl(entry.path));
                html += `<tr>
                    <td class="file-name">📄 <a href="${url}" target="_blank" rel="noopener">${name}</a></td>
                    <td class="file-size">${formatFileSize(entry.size)}</td>
                    <td class="file-actions">
                        <a class="btn btn-sm" href="${url}" download="${name}">⬇️ Download</a>
                    </td>
                </tr>`;
            }
        });

        html += '</tbody></table>';
        fileList.innerHTML = html;
    };

    fileList.addEventListener('click', (e) => {
        const dirLink = e.target.closest('.dir-link');
        if (dirLink) {
            e.preventDefault();
            loadDirectory(dirLink.dataset.path);
            return;
        }

        const zipButton = e.target.closest('.zip-btn');
        if (zipButton) {
            downloadFolder(zipButton.dataset.path);
            return;
        }

        const uploadHereButton = e.target.closest('.upload-here-btn');
        if (uploadHereButton) {
            openUploadDialog(uploadHereButton.dataset.path);
        }
    });

    // Format file size
    const formatFileSize = (bytes) => {
        if (bytes === 0) return '0 B';
        const k = 1024;
        const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
        const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1);
        return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i];
    };

    // Download folder as ZIP. The server streams the zip as it builds it and the browser
    // saves it straight to disk; this polls the server to show how far through it is.
    const downloadFolder = async (path) => {
        const folderName = path.split('/').filter(Boolean).pop() || 'folder';

        let operationId;
        try {
            const initResponse = await fetch(`/api/zip/init?path=${encodeURIComponent(path)}`);
            const initData = initResponse.ok ? await initResponse.json() : null;
            if (!initData || !initData.success) {
                throw new Error('Failed to initialize ZIP operation');
            }
            operationId = initData.operationId;
        } catch (error) {
            console.error('Download error:', error);
            alert('Failed to download folder');
            return;
        }

        const statusDiv = document.createElement('div');
        statusDiv.className = 'download-status';
        statusDiv.innerHTML = `
            <h4>📦 ${escapeHtml(folderName)}.zip</h4>
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

        const finish = (message, isError) => {
            currentFile.textContent = message;
            currentFile.classList.toggle('error', isError);
            setTimeout(() => statusDiv.remove(), isError ? 5000 : 3000);
        };

        const link = document.createElement('a');
        link.href = `/api/download/folder?path=${encodeURIComponent(path)}&operation_id=${encodeURIComponent(operationId)}`;
        link.download = `${folderName}.zip`;
        document.body.appendChild(link);
        link.click();
        link.remove();

        let failedPolls = 0;
        const poll = async () => {
            try {
                const response = await fetch(`/api/zip/progress?id=${encodeURIComponent(operationId)}`);
                if (response.status === 404) {
                    // Finished long ago and cleaned up
                    finish('✅ Download complete', false);
                    return;
                }
                if (!response.ok) {
                    throw new Error(`Server responded ${response.status}`);
                }
                const progress = await response.json();
                failedPolls = 0;

                // Files are counted while zipping, so the total shows up after a moment
                progressBar.style.width = `${progress.percentage}%`;
                progressText.textContent = progress.total_files > 0
                    ? `${Math.round(progress.percentage)}% (${progress.processed_files.toLocaleString()} of ${progress.total_files.toLocaleString()} files)`
                    : `${progress.processed_files.toLocaleString()} files`;
                currentFile.textContent = progress.current_file || 'Preparing download...';

                if (progress.failed) {
                    finish(`❌ ${progress.current_file || 'Failed to create ZIP archive'}`, true);
                    return;
                }
                if (progress.done) {
                    finish('✅ Download complete', false);
                    return;
                }
            } catch (error) {
                console.error('Progress polling error:', error);
                if (++failedPolls >= 20) {
                    finish('❌ Lost contact with the server', true);
                    return;
                }
            }
            setTimeout(poll, 250);
        };
        poll();
    };

    // Upload: files or whole folders, into the folder being viewed or any folder in the
    // list. Uses XMLHttpRequest because fetch can't report upload progress.
    const uploadBtn = document.getElementById('uploadBtn');
    const uploadModal = document.getElementById('uploadModal');
    const closeModal = document.querySelector('.close');
    const uploadArea = document.getElementById('uploadArea');
    const fileInput = document.getElementById('fileInput');
    const folderInput = document.getElementById('folderInput');
    const pickFolderBtn = document.getElementById('pickFolderBtn');
    const uploadProgress = document.getElementById('uploadProgress');
    const uploadDestination = document.getElementById('uploadDestination');
    const dropOverlay = document.getElementById('dropOverlay');
    const progressBar = uploadProgress.querySelector('.progress-bar');
    const progressText = uploadProgress.querySelector('.progress-text');
    const uploadStatus = uploadProgress.querySelector('.upload-status');

    let uploadTarget = '';
    let uploading = false;

    const describeFolder = (path) => ['🏠 Home', ...path.split('/').filter(Boolean)].join(' / ');
    const isDialogOpen = () => uploadModal.style.display === 'block';

    const openUploadDialog = (path) => {
        // While an upload runs the dialog shows its progress, so keep its destination
        if (!uploading) {
            uploadTarget = path;
            uploadDestination.textContent = describeFolder(path);
        }
        uploadModal.style.display = 'block';
    };

    uploadBtn.addEventListener('click', () => openUploadDialog(currentPath));

    closeModal.addEventListener('click', () => {
        uploadModal.style.display = 'none';
    });

    window.addEventListener('click', (event) => {
        if (event.target === uploadModal) {
            uploadModal.style.display = 'none';
        }
    });

    // Picking folders isn't available everywhere (e.g. iOS)
    if (!('webkitdirectory' in folderInput)) {
        pickFolderBtn.hidden = true;
    }

    // Clicking the drop area (or its "Choose files" button) picks files
    uploadArea.addEventListener('click', (e) => {
        if (e.target.closest('#pickFolderBtn')) {
            folderInput.click();
        } else {
            fileInput.click();
        }
    });

    fileInput.addEventListener('change', () => {
        uploadItems([...fileInput.files].map(file => ({ file, path: file.name })));
    });

    folderInput.addEventListener('change', () => {
        // webkitRelativePath starts with the chosen folder's own name, so it's recreated
        uploadItems([...folderInput.files].map(file => ({ file, path: file.webkitRelativePath || file.name })));
    });

    // Reads dropped files and folders (recursively) into { file, path } items
    const readDropped = async (dataTransfer) => {
        // Entries must be taken synchronously, before the drop event has finished
        const entries = [...dataTransfer.items]
            .filter(item => item.kind === 'file')
            .map(item => item.webkitGetAsEntry && item.webkitGetAsEntry());
        if (entries.length === 0 || !entries.every(Boolean)) {
            // No folder support in this browser: plain files only
            return [...dataTransfer.files].map(file => ({ file, path: file.name }));
        }

        const items = [];
        const visit = async (entry) => {
            if (entry.isFile) {
                const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
                items.push({ file, path: entry.fullPath.replace(/^\/+/, '') });
            } else if (entry.isDirectory) {
                const reader = entry.createReader();
                // Each call returns the next batch of entries, then an empty one at the end
                while (true) {
                    const batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
                    if (batch.length === 0) {
                        break;
                    }
                    for (const child of batch) {
                        await visit(child);
                    }
                }
            }
        };
        for (const entry of entries) {
            await visit(entry);
        }
        return items;
    };

    // Files can be dropped anywhere on the page: into the open dialog's folder, otherwise
    // into the folder being viewed. This also stops a stray drop from navigating away.
    const hasFiles = (e) => e.dataTransfer && [...e.dataTransfer.types].includes('Files');
    let hideDropHintsTimer;
    const hideDropHints = () => {
        dropOverlay.classList.remove('visible');
        uploadArea.classList.remove('dragover');
    };

    document.addEventListener('dragover', (e) => {
        if (!hasFiles(e)) {
            return;
        }
        e.preventDefault();
        if (isDialogOpen()) {
            uploadArea.classList.add('dragover');
        } else {
            dropOverlay.querySelector('strong').textContent = describeFolder(currentPath);
            dropOverlay.classList.add('visible');
        }
        // dragover keeps firing while something is held over the page
        clearTimeout(hideDropHintsTimer);
        hideDropHintsTimer = setTimeout(hideDropHints, 150);
    });

    document.addEventListener('drop', async (e) => {
        if (!hasFiles(e)) {
            return;
        }
        e.preventDefault();
        hideDropHints();
        if (!isDialogOpen()) {
            openUploadDialog(currentPath);
        }
        if (uploading) {
            return;
        }
        uploadItems(await readDropped(e.dataTransfer));
    });

    const uploadItems = (items) => {
        if (uploading) {
            return;
        }
        if (items.length === 0) {
            alert('There are no files to upload (empty folders are skipped)');
            return;
        }
        uploading = true;

        const totalBytes = items.reduce((sum, item) => sum + item.file.size, 0);
        const fileCount = (count) => `${count.toLocaleString()} file${count === 1 ? '' : 's'}`;

        const formData = new FormData();
        // The name carries each file's path within a chosen or dropped folder
        items.forEach(({ file, path }) => formData.append('file', file, path));

        const setProgress = (percent, text) => {
            progressBar.style.width = `${percent}%`;
            progressText.textContent = text;
        };

        const resetAfter = (delay) => {
            setTimeout(() => {
                uploading = false;
                uploadArea.style.display = 'block';
                uploadProgress.style.display = 'none';
                setProgress(0, '0%');
                // Allows picking the same files again
                fileInput.value = '';
                folderInput.value = '';
                // Also after a failure, as some files may have been saved
                loadDirectory(currentPath);
            }, delay);
        };

        uploadArea.style.display = 'none';
        uploadProgress.style.display = 'block';
        uploadStatus.textContent = `Uploading ${fileCount(items.length)} (${formatFileSize(totalBytes)})...`;
        setProgress(0, '0%');

        const xhr = new XMLHttpRequest();
        xhr.open('POST', `/api/upload?path=${encodeURIComponent(uploadTarget)}`);

        xhr.upload.addEventListener('progress', (e) => {
            if (e.lengthComputable) {
                const percent = e.loaded / e.total * 100;
                setProgress(percent, `${Math.round(percent)}% · ${formatFileSize(e.loaded)} of ${formatFileSize(e.total)}`);
            }
        });

        const fail = (message) => {
            console.error('Upload error:', message);
            uploadStatus.textContent = '❌ Upload failed: ' + message;
            resetAfter(2500);
        };

        xhr.addEventListener('load', () => {
            let result = null;
            try {
                result = JSON.parse(xhr.responseText);
            } catch (error) {
                // Not JSON, handled below
            }

            if (xhr.status < 200 || xhr.status >= 300 || !result || !result.success) {
                fail((result && result.message) || `Server responded ${xhr.status}`);
                return;
            }

            setProgress(100, '100%');
            uploadStatus.textContent = `✅ Uploaded ${fileCount(result.count)}`;
            setTimeout(() => {
                uploadModal.style.display = 'none';
            }, 1500);
            resetAfter(1500);
        });

        xhr.addEventListener('error', () => fail('Network error'));

        xhr.send(formData);
    };

    // Stop server button
    const stopBtn = document.getElementById('stopBtn');
    stopBtn.addEventListener('click', async () => {
        if (confirm('Are you sure you want to stop the server?')) {
            try {
                const response = await fetch('/api/stop', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ confirm: true })
                });
                const result = await response.json();
                alert(result.success ? 'Server is shutting down...' : result.message);
            } catch (error) {
                console.error('Error stopping server:', error);
                alert('Failed to stop server');
            }
        }
    });

    // Load initial directory
    loadDirectory();
});
