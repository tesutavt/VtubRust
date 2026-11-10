## VtubRust 0.5.0: no VTube Studio needed any more

### 🎥 Built-in face tracker
- The webcam face tracker is now part of rigrs: a Rust port of OpenSeeFace running on ONNX
  Runtime, with camera capture through Windows Media Foundation. No `facetracker.exe`, no
  Python, no VTube Studio install. The tracker files ship in the zip.
- Same quality levels and the same numbers as before, so existing calibration keeps working.
- Switch: Settings → Camera → "Built-in tracker". If it cannot start, rigrs goes back to
  `facetracker.exe` by itself. Hand tracking and Mediapipe still use `facetracker.exe`.

### 📦 Cubism Core download
- No `Live2DCubismCore.dll`? rigrs offers to download the official Cubism SDK from live2d.com
  (after you accept the Live2D licence) and takes the Core library from it.

### 📦 How to update
rigrs 0.3.2 and newer offers the update by itself ("Update and restart"). Otherwise download
`VtubRust-v0.5.0-windows-x64.zip`, unzip it and run `rigrs.exe`.

This is an educational fan project, not affiliated with VTube Studio / DenchiSoft, and it does
not use their code. Live2D Cubism Core is the property of Live2D Inc. and is not included.
Third-party licences: `THIRD_PARTY.txt`.
