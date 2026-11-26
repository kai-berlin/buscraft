# Commits this folder and pushes it to GitHub (git@github.com:kai-berlin/buscraft.git).
# Needs Git for Windows (https://git-scm.com/download/win, default options).
# Start it with a double-click on publish-to-github.cmd.

$remote = 'git@github.com:kai-berlin/buscraft.git'
$message = 'BusCraft: KNX planning and commissioning tool'
Set-Location $PSScriptRoot

function Stop-With($text) {
    Write-Host ""
    Write-Host $text -ForegroundColor Red
    exit 1
}

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Start-Process 'https://git-scm.com/download/win'
    Stop-With "Git is not installed. Install it from https://git-scm.com/download/win and run this again."
}

# 1. Local repository and commit
if (-not (Test-Path '.git')) {
    git init | Out-Null
}
if (-not (git config user.name)) { git config user.name 'kai-berlin' }
if (-not (git config user.email)) { git config user.email 'kai-berlin@users.noreply.github.com' }
git add -A
git diff --cached --quiet
if ($LASTEXITCODE -ne 0) {
    git commit -m $message | Out-Null
    Write-Host "Committed the current state." -ForegroundColor Green
} else {
    Write-Host "Nothing new to commit."
}
git branch -M main

# 2. Remote
$existing = git remote 2>$null
if ($existing -contains 'origin') { git remote set-url origin $remote } else { git remote add origin $remote }

# 3. SSH key for GitHub
$key = Join-Path $env:USERPROFILE '.ssh\id_ed25519'
if (-not (Test-Path "$key.pub")) {
    Write-Host ""
    Write-Host "Creating an SSH key for GitHub. Press Enter at the questions (or choose a passphrase)."
    New-Item -ItemType Directory -Force (Split-Path $key) | Out-Null
    ssh-keygen -t ed25519 -f $key -C 'buscraft'
}
$env:GIT_SSH_COMMAND = 'ssh -o StrictHostKeyChecking=accept-new'
ssh -o StrictHostKeyChecking=accept-new -o BatchMode=yes -T git@github.com 2>&1 | Out-Null
if ($LASTEXITCODE -ne 1) {
    # GitHub answers with exit code 1 when the key is accepted (it offers no shell).
    Get-Content "$key.pub" | Set-Clipboard
    Write-Host ""
    Write-Host "GitHub doesn't know this computer's key yet." -ForegroundColor Yellow
    Write-Host "The key is in your clipboard. A browser page opens: paste it under 'Key', give it a title, click 'Add SSH key'."
    Start-Process 'https://github.com/settings/ssh/new'
    Read-Host "Press Enter when the key is added"
}

# 4. Push (merging what is already on GitHub, e.g. a README made there)
git fetch origin 2>$null
git rev-parse --verify --quiet origin/main | Out-Null
if ($LASTEXITCODE -eq 0) {
    git merge origin/main --allow-unrelated-histories -m 'Merge the GitHub repository'
    if ($LASTEXITCODE -ne 0) { Stop-With "The files on GitHub conflict with the local ones. Nothing was pushed." }
}
git push -u origin main
if ($LASTEXITCODE -ne 0) { Stop-With "Pushing failed - see the message above." }
Write-Host ""
Write-Host "Done: the project is on https://github.com/kai-berlin/buscraft" -ForegroundColor Green
