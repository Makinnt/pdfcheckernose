# Instalador Windows

## Local

```powershell
.\installer\windows\stage.ps1   # release + dist/windows (+ JRE Temurin)
iscc installer\windows\lexpdf.iss
```

Sale `dist/lexpdf-setup.exe`. Instala en ruta corta (`C:\lexpdf`)
por los paths largos de LT.

## Java

El instalador NO exige Java: al arrancar comprueba `jre\bin\java.exe`
empaquetado y luego el del sistema. Si no hay ninguno, ofrece abrir la
descarga de Temurin 21 y deja continuar. La app busca java en:
`$JAVA_BIN` → `jre\` junto al exe → `PATH`.

## Firma gratis (SignPath, solo OSS)

1. Cuenta en **signpath.io**, proyecto `lexpdf`, repo vinculado
   (el repo debe ser público).
2. Secrets en GitHub: `SIGNPATH_API_TOKEN`, `SIGNPATH_ORG_ID`.
3. Descomenta el paso en `.github/workflows/release.yml` y ajusta slugs.
4. Sin firmar, SmartScreen avisará al principio (se atenúa con descargas).

## CI

Push de tag `v*` → build release → Inno → release en GitHub con el setup.
Sin tag, el workflow deja el instalador como artifact.
