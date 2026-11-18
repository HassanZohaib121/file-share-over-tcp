param(
    [string]$Device
)

$ErrorActionPreference = "Stop"
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$target = "aarch64-linux-android"
$applicationId = "com.rustdrop.app"

Set-Location $repositoryRoot

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Cargo was not found on PATH."
}
if (-not (Get-Command adb -ErrorAction SilentlyContinue)) {
    throw "ADB was not found on PATH. Install Android SDK Platform-Tools and add its platform-tools folder to PATH."
}

$runArguments = @("apk2", "run", "--lib", "--target", $target, "--no-logcat")
if ($Device) {
    $runArguments += @("--device", $Device)
}

function Get-ProjectSnapshot {
    $inputs = @()
    foreach ($directory in @("src", "ui")) {
        $inputs += Get-ChildItem -LiteralPath (Join-Path $repositoryRoot $directory) -Recurse -File
    }
    foreach ($file in @("Cargo.toml", "Cargo.lock", "build.rs")) {
        $path = Join-Path $repositoryRoot $file
        if (Test-Path -LiteralPath $path) {
            $inputs += Get-Item -LiteralPath $path
        }
    }
    return (($inputs | Sort-Object FullName | ForEach-Object {
        "{0}|{1}|{2}" -f $_.FullName, $_.Length, $_.LastWriteTimeUtc.Ticks
    }) -join "`n")
}

function Start-AndroidApp {
    param([bool]$InitialRun = $false)

    if (-not $InitialRun) {
        if ($Device) {
            & adb -s $Device shell am force-stop $applicationId
        }
        else {
            & adb shell am force-stop $applicationId
        }
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "Could not force-stop the app before reinstalling."
        }
    }

    & cargo @runArguments
    if ($LASTEXITCODE -ne 0) {
        if ($InitialRun) {
            throw "Android build/run failed with exit code $LASTEXITCODE."
        }
        Write-Warning "Android build/run failed with exit code $LASTEXITCODE. Watching for the next change."
    }
}

Write-Host "Building and launching RustDrop on Android..."
Start-AndroidApp -InitialRun $true

Write-Host "Watching src/, ui/, Cargo.toml, Cargo.lock, and build.rs. Press Ctrl+C to stop."
$snapshot = Get-ProjectSnapshot
while ($true) {
    Start-Sleep -Milliseconds 500
    $currentSnapshot = Get-ProjectSnapshot
    if ($currentSnapshot -ne $snapshot) {
        Start-Sleep -Milliseconds 700
        $currentSnapshot = Get-ProjectSnapshot
        if ($currentSnapshot -ne $snapshot) {
            $snapshot = $currentSnapshot
            Write-Host "Change detected. Rebuilding and relaunching RustDrop..."
            Start-AndroidApp
        }
    }
}
