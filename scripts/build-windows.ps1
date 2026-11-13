$ErrorActionPreference = "Stop"

$repositoryDirectory = Split-Path -Parent $PSScriptRoot
$dx = Join-Path $HOME ".cargo\bin\dx.exe"

if (-not (Test-Path -LiteralPath $dx -PathType Leaf)) {
    throw "Dioxus CLI not found: $dx"
}

Push-Location -LiteralPath $repositoryDirectory

try {
    & $dx bundle `
        --desktop `
        --release `
        --package wallet `
        --package-types msi `
        --no-default-features `
        --features mainnet

    $buildExitCode = $LASTEXITCODE
}
finally {
    Pop-Location
}

exit $buildExitCode
