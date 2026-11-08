param(
    [Parameter(Mandatory)][int]$OrbitProcess,
    [Parameter(Mandatory)][string]$FilePath
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes

$scope = [System.Windows.Automation.TreeScope]
$element = [System.Windows.Automation.AutomationElement]
$processCondition = [System.Windows.Automation.PropertyCondition]::new($element::ProcessIdProperty, $OrbitProcess)
$dialogCondition = [System.Windows.Automation.PropertyCondition]::new($element::ClassNameProperty, '#32770')
$condition = [System.Windows.Automation.AndCondition]::new($processCondition, $dialogCondition)
$fileNameIds = [System.Windows.Automation.OrCondition]::new([System.Windows.Automation.Condition[]]@(
    [System.Windows.Automation.PropertyCondition]::new($element::AutomationIdProperty, '1148'),
    [System.Windows.Automation.PropertyCondition]::new($element::AutomationIdProperty, '1001')
))
$valueCondition = [System.Windows.Automation.PropertyCondition]::new($element::IsValuePatternAvailableProperty, $true)
$fileNameCondition = [System.Windows.Automation.AndCondition]::new($fileNameIds, $valueCondition)
$acceptCondition = [System.Windows.Automation.PropertyCondition]::new($element::AutomationIdProperty, '1')

$deadline = (Get-Date).AddSeconds(15)
do {
    $dialog = $element::RootElement.FindFirst($scope::Children, $condition)
    if ($dialog) {
        $fileName = $dialog.FindFirst($scope::Descendants, $fileNameCondition)
        $accept = $dialog.FindFirst($scope::Descendants, $acceptCondition)
        if ($fileName -and $accept) {
            $fileName.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($FilePath)
            $accept.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
            return
        }
    }
    Start-Sleep -Milliseconds 200
} while ((Get-Date) -lt $deadline)

throw 'The Orbit file dialog did not expose a filename field and an accept button.'
