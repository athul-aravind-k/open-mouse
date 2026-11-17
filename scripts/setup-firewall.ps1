<#
.SYNOPSIS
    Configures Windows Defender Firewall for OpenMouse.
.DESCRIPTION
    Creates inbound firewall rules for:
    - UDP 48898: OpenMouse LAN Auto-Discovery
    - UDP 48899: OpenMouse QUIC Transport
    Configures rules across Domain, Private, and Public network profiles.
    Automatically requests UAC elevation if not run as Administrator.
#>

# Check for Administrator privileges
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $isAdmin) {
    Write-Host "OpenMouse Firewall Setup requires Administrator privileges." -ForegroundColor Yellow
    Write-Host "Requesting UAC elevation..." -ForegroundColor Cyan
    try {
        Start-Process powershell -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
        exit 0
    } catch {
        Write-Error "Failed to elevate process: $_"
        exit 1
    }
}

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "   OpenMouse Windows Firewall Setup     " -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

$rules = @(
    @{
        DisplayName = "OpenMouse LAN Discovery"
        Description = "Inbound UDP discovery beacons for OpenMouse peer auto-detection"
        Protocol    = "UDP"
        LocalPort   = 48898
    },
    @{
        DisplayName = "OpenMouse QUIC Transport"
        Description = "Inbound QUIC transport for OpenMouse low-latency input and clipboard sharing"
        Protocol    = "UDP"
        LocalPort   = 48899
    }
)

$profiles = "Domain,Private,Public"

foreach ($rule in $rules) {
    Write-Host "Configuring rule: $($rule.DisplayName)..." -NoNewline
    
    # Remove existing rule to prevent duplicates
    $existing = Get-NetFirewallRule -DisplayName $rule.DisplayName -ErrorAction SilentlyContinue
    if ($existing) {
        $existing | Remove-NetFirewallRule
    }
    
    # Create new rule
    try {
        New-NetFirewallRule `
            -DisplayName $rule.DisplayName `
            -Description $rule.Description `
            -Direction Inbound `
            -Action Allow `
            -Protocol $rule.Protocol `
            -LocalPort $rule.LocalPort `
            -Profile Domain, Private, Public `
            -Enabled True | Out-Null
            
        Write-Host " [SUCCESS]" -ForegroundColor Green
        Write-Host "  Port: $($rule.LocalPort)/$($rule.Protocol) (Profiles: $profiles)" -ForegroundColor Gray
    } catch {
        Write-Host " [FAILED]" -ForegroundColor Red
        Write-Error "Failed to add firewall rule $($rule.DisplayName): $_"
    }
}

Write-Host ""
Write-Host "Firewall configuration completed successfully." -ForegroundColor Green
Write-Host "OpenMouse can now communicate over UDP 48898 and 48899 on your local network." -ForegroundColor Gray
Write-Host ""
Write-Host "Press any key to exit..."
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
