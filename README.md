# Clipboard Typer

A small Windows tray application that reads Unicode text from the clipboard and types it into the target window with simulated keyboard input. It does not use the target application's paste command, so it also works in fields where `Ctrl+V` is unavailable.

Ordinary Windows targets use physical keystrokes for ordinary and Shift characters, plus Unicode input for AltGr-dependent characters such as `[`, `]`, and `$`. This avoids the modifier loss seen in remote and recovery consoles while preserving parentheses and other Shift punctuation. Hyper-V VMConnect continues to use its dedicated keyboard interface.

When the selected window is Hyper-V VMConnect, Clipboard Typer uses the Hyper-V `Msvm_Keyboard.TypeText` interface instead. This preserves layout-sensitive PowerShell syntax such as `$`, `[`, `]`, quotes, and colons. Clipboard text is passed to the helper through standard input and is never embedded in its command line.

Clipboard Typer starts normally without an administrator prompt. It requests elevation only after selecting Hyper-V VMConnect or another window whose owning process is elevated. Normal target windows remain prompt-free.

The application and tray use an embedded crosshair icon with a green center. The tray automatically uses a dark reticle with the light Windows system theme and a white reticle with the dark system theme.

## Download

Download the Windows executable from the repository's [Releases](https://github.com/fredrikwallander-cell/clipboard-typer/releases) page. Release binaries are intentionally unsigned and are not stored in the source tree.

## Use

1. Start `clipboard-typer.exe`. It runs without a visible window.
2. Focus the field that should receive the text.
3. Press **Ctrl+Alt+V** to type into the currently focused field.
4. Left-click the tray icon to enter **Type clipboard on next click** mode. Alternatively, choose that command from the right-click menu. Move to the desired field; the window is tinted green to show the current target. Click once to place the caret and type the clipboard text.
5. Right-click the tray icon and select **Exit** to close the utility.

Both tray targeting paths wait for a natural click in the destination field, so opening the tray cannot change where the text is typed.

## Build

Use a Visual Studio Developer PowerShell with the Desktop development with C++ workload installed:

```powershell
cargo build --release
```

The executable is written to `target\release\clipboard-typer.exe`.

## Notes

- Unicode text and multi-line text are supported.
- Choose **Typing delay** from the tray menu to select 0, 10, 20, 50, or 100 ms between characters. The default is 20 ms.
- Press **Escape** to stop typing immediately or cancel **Type clipboard on next click** mode.
- Windows prevents lower-integrity applications from injecting input into elevated applications, so Clipboard Typer starts an on-demand elevated helper only when the selected target requires it.
- The global hotkey is currently `Ctrl+Alt+V`.
- Hotkey typing waits for Ctrl and Alt to be released, preventing those modifiers from affecting the simulated characters.
