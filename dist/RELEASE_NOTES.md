## VtubRust 0.5.0: works without VTube Studio

### 🚪 Leaving VTube Studio
- New card **Settings → General → "Working without VTube Studio"** shows what rigrs still takes
  from a VTube Studio install and replaces it in three steps:
  1. **Cubism Core**: download your own copy from live2d.com (official Cubism SDK; you accept
     the Live2D licence).
  2. **Face tracker**: the built-in one.
  3. **"Move library to rigrs"**: copies your models, items and backgrounds into the rigrs data
     folder. Model positions, hotkeys, ArtMesh settings, pinned items and the background keep
     working; nothing in VTube Studio is changed or deleted. The app restarts at the end.
- After the three steps VTube Studio can be uninstalled. Hand tracking, the Mediapipe tracker
  and the NVIDIA tracker still need files from VTube Studio.

### 🎥 Built-in face tracker
- A Rust port of OpenSeeFace on ONNX Runtime with camera capture through Windows Media
  Foundation: no `facetracker.exe`, no Python. All five quality levels ship in the archive.
- If it cannot start and a `facetracker.exe` exists, rigrs falls back to it.

### 🛠 Fixed since the 0.5.0 pre-release
- The built-in tracker did not start at most quality levels (the archive held the model of
  one level only, so rigrs silently went back to `facetracker.exe`). A missing level now
  falls back to the nearest installed one.
- "Tracker exited (exit code: 0)": a tracker process left behind by a crashed or killed rigrs
  kept the camera open. Tracker processes now always close together with rigrs.
- The "Download Cubism Core" button was only shown when the library was missing.

### 📦 How to update
rigrs 0.3.2 and newer offers the update by itself ("Update and restart"). Otherwise download
`VtubRust-v0.5.0-windows-x64.zip`, unzip it and run `rigrs.exe`.

This is an educational fan project, not affiliated with VTube Studio / DenchiSoft, and it does
not use their code. Live2D Cubism Core is the property of Live2D Inc. and is not included.
Third-party licences: `THIRD_PARTY.txt`.
