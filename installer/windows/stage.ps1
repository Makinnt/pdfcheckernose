# Arma dist/windows/ para el instalador (o el tarball de prueba).
# Uso: .\installer\windows\stage.ps1
# Requiere: Rust estable + Java 17+ (build.rs descarga LT solo la 1ª vez).
$ErrorActionPreference = 'Stop'
$Root = Resolve-Path (Join-Path $PSScriptRoot '../..')
Set-Location $Root

cargo build --release
New-Item -ItemType Directory -Force dist/windows | Out-Null
Copy-Item target/release/pdf-corrector.exe dist/windows/
Copy-Item assets/pdfium/pdfium.dll dist/windows/
if (Test-Path assets/lt/languagetool-server.jar) {
  Copy-Item assets/lt dist/windows/lt -Recurse -Force
} else {
  Write-Warning 'Sin assets/lt (¿falló la descarga en build?). El instalador pedirá Java igualmente.'
}
# JRE empaquetado (opcional pero recomendado): Temurin 21 x64
if (-not (Test-Path dist/windows/jre/bin/java.exe)) {
  Write-Host 'Descargando Temurin JRE 21 x64...'
  $api = 'https://api.adoptium.net/v3/binary/latest/21/ga/windows/x64/jre/hotspot/normal/eclipse'
  Invoke-WebRequest $api -OutFile dist/temurin-jre.zip
  Expand-Archive dist/temurin-jre.zip -DestinationPath dist/tmp-jre -Force
  $inner = Get-ChildItem dist/tmp-jre | Select-Object -First 1
  Move-Item $inner.FullName dist/windows/jre -Force
  Remove-Item dist/tmp-jre -Recurse -Force
  Remove-Item dist/temurin-jre.zip -Force
}
Write-Host 'OK: dist/windows listo para iscc installer/windows/pdf-corrector.iss'
