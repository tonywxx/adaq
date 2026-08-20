$ErrorActionPreference = 'Stop'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$installation = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $installation) {
  throw 'Visual Studio with the x64 C++ toolchain was not found.'
}

$vsDevCmd = Join-Path $installation 'Common7\Tools\VsDevCmd.bat'
$environment = & $env:COMSPEC /s /c "`"$vsDevCmd`" -no_logo -arch=x64 -host_arch=x64 && set"
if ($LASTEXITCODE -ne 0) {
  throw "VsDevCmd failed with exit code $LASTEXITCODE."
}

$exportedVariables = @(
  'INCLUDE',
  'LIB',
  'LIBPATH',
  'Path',
  'UCRTVersion',
  'UniversalCRTSdkDir',
  'VCINSTALLDIR',
  'VCToolsInstallDir',
  'VCToolsRedistDir',
  'VSCMD_ARG_HOST_ARCH',
  'VSCMD_ARG_TGT_ARCH',
  'VSINSTALLDIR',
  'WindowsLibPath',
  'WindowsSdkBinPath',
  'WindowsSdkDir',
  'WindowsSDKVersion'
)

foreach ($line in $environment) {
  if ($line -match '^([^=]+)=(.*)$' -and $exportedVariables -contains $Matches[1]) {
    "$($Matches[1])=$($Matches[2])" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
  }
}

# Git Bash prepends its incomplete MSYS Perl, which cannot configure vendored OpenSSL.
$strawberryPerl = 'C:\Strawberry\perl\bin\perl.exe'
if (-not (Test-Path $strawberryPerl)) {
  throw 'Strawberry Perl was not found.'
}
"OPENSSL_SRC_PERL=$strawberryPerl" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append

# Git Bash also shadows the MSVC linker with its unrelated Unix link.exe.
$linker = Get-ChildItem "$installation\VC\Tools\MSVC\*\bin\Hostx64\x64\link.exe" |
  Sort-Object FullName |
  Select-Object -Last 1
if (-not $linker) {
  throw 'MSVC linker was not found.'
}
"CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=$($linker.FullName)" |
  Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
