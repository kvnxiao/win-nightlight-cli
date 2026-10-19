$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$windows = 0
foreach ($process in @(Get-Process -Name SystemSettings -ErrorAction SilentlyContinue)) {
    $element = [Windows.Automation.AutomationElement]::RootElement.FindFirst(
        [Windows.Automation.TreeScope]::Descendants,
        [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $process.Id))
    if ($element) { $windows += 1 }
}
$windows
