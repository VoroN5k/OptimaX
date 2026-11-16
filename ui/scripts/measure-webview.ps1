# Заміри витрат інтерфейсу OptimaX (docs/DESIGN.md розд. 10, етап 7 редизайну): CPU, GPU й пам'ять
# процесу optimax.exe разом з усіма його процесами WebView2 (msedgewebview2.exe — нащадки).
# Нічого не змінює, лише читає лічильники Windows.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File ui\scripts\measure-webview.ps1 -Seconds 30
#
# CPU — частка одного логічного ядра (100 % = одне ядро повністю) і частка всього процесора.
# GPU — сума «\GPU Engine(pid_*)\Utilization Percentage» цих процесів (як у диспетчері завдань).
# Пам'ять — приватні байти наприкінці заміру.
param(
  [int]$Seconds = 30,
  [string]$Label = ""
)
$ErrorActionPreference = "Stop"

$root = Get-Process -Name optimax -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $root) { Write-Output "Вікно OptimaX не знайдено."; exit 1 }

# Дерево процесів: optimax.exe і всі нащадки (WebView2 запускає браузерний, GPU-, рендер-процеси).
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
$ids = [System.Collections.Generic.HashSet[int]]::new()
[void]$ids.Add($root.Id)
do {
  $added = $false
  foreach ($p in $all) {
    if ($ids.Contains([int]$p.ParentProcessId) -and -not $ids.Contains([int]$p.ProcessId)) {
      [void]$ids.Add([int]$p.ProcessId); $added = $true
    }
  }
} while ($added)

function CpuTotal([System.Collections.Generic.HashSet[int]]$set) {
  $sum = 0.0
  foreach ($id in $set) {
    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($p) { $sum += $p.TotalProcessorTime.TotalMilliseconds }
  }
  return $sum
}

$gpuPaths = $ids | ForEach-Object { "\GPU Engine(pid_$($_)_*)\Utilization Percentage" }
$cores = [Environment]::ProcessorCount
$cpu0 = CpuTotal $ids
$t0 = [Diagnostics.Stopwatch]::StartNew()
$gpuSamples = @()
$n = [Math]::Max(1, [int]($Seconds))
for ($i = 0; $i -lt $n; $i++) {
  try {
    $s = (Get-Counter -Counter $gpuPaths -SampleInterval 1 -MaxSamples 1 -ErrorAction SilentlyContinue).CounterSamples
    $gpuSamples += (($s | Measure-Object -Property CookedValue -Sum).Sum)
  } catch {
    Start-Sleep -Seconds 1
  }
}
$elapsed = $t0.Elapsed.TotalMilliseconds
$cpu1 = CpuTotal $ids
$private = 0
foreach ($id in $ids) {
  $p = Get-Process -Id $id -ErrorAction SilentlyContinue
  if ($p) { $private += $p.PrivateMemorySize64 }
}
$core = 100.0 * ($cpu1 - $cpu0) / $elapsed
$gpu = if ($gpuSamples.Count) { ($gpuSamples | Measure-Object -Average).Average } else { 0 }
"{0}процесів: {1}; {2:N0} с; CPU {3:N2} % ядра ({4:N2} % процесора, {5} ядер); GPU {6:N2} %; пам'ять {7:N0} МБ" -f `
  ($(if ($Label) { "[$Label] " } else { "" })), $ids.Count, ($elapsed / 1000), $core, ($core / $cores), $cores, $gpu, ($private / 1MB)
