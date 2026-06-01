param(
    [string]$Package = "demo-servo-winit",
    [int]$Seconds = 15,
    [string]$Url = ""
)

$ErrorActionPreference = "Stop"

function Stop-ProcessTree {
    param([int]$ProcessId)

    $children = Get-CimInstance Win32_Process -Filter "ParentProcessId = $ProcessId"
    foreach ($child in $children) {
        Stop-ProcessTree -ProcessId $child.ProcessId
    }

    $process = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($process) {
        Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
    }
}

$repo = Resolve-Path (Join-Path $PSScriptRoot "..")
$cargoArgs = @("run", "-p", $Package)
if ($Url) {
    $cargoArgs += "--"
    $cargoArgs += $Url
}

Write-Host "Starting: cargo $($cargoArgs -join ' ')"
$process = Start-Process -FilePath "cargo" -ArgumentList $cargoArgs -WorkingDirectory $repo -PassThru

try {
    if ($process.WaitForExit($Seconds * 1000)) {
        exit $process.ExitCode
    }

    Write-Host "Smoke window survived $Seconds seconds; stopping process tree."
    Stop-ProcessTree -ProcessId $process.Id
    exit 0
}
finally {
    Stop-ProcessTree -ProcessId $process.Id
}