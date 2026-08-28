param(
    # Open a specific demo page, e.g. ./dev.ps1 -DemoPage home
    [string]$DemoPage = ""
)

# A survivor instance in the tray would swallow the next launch
# (single-instance guard), so always start from a clean slate.
Stop-Process -Name fastpotify -ErrorAction SilentlyContinue

if ($DemoPage) {
    cargo watch -x "run --features demo -- --demo --demo-page $DemoPage"
} else {
    cargo watch -x "run --features demo -- --demo"
}
