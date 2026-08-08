# Windows WASAPI manual test checklist

Run these from a Windows development shell after `cargo build`.

1. Build with `cargo build`.
2. Start system-only capture:

   ```powershell
   cargo run -- start "Windows system audio"
   ```

   Play a YouTube or meeting recording through the default Windows output,
   stop with Ctrl+C, and verify that the session contains `system.wav` only.

3. Start microphone-only capture:

   ```powershell
   cargo run -- start "Windows microphone" --mic-only
   ```

   Speak into the default microphone, stop with Ctrl+C, and verify that the
   session contains `mic.wav` only.

4. Start combined capture:

   ```powershell
   cargo run -- start "Windows both" --mic
   ```

   Play system audio while speaking, stop with Ctrl+C, and verify that both
   WAV files exist and remain separate.

5. Disable microphone privacy access or disconnect an endpoint and confirm
   Rusteze reports a useful startup error without leaving an active capture
   process.
