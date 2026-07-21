# Factory Presets

This page explains how to import Sequential's official factory sound banks into
the synth-app. The synth supports both the Prophet Rev2 and Prophet '08 SysEx
formats.

## Downloading the Presets

Sequential publishes the factory sound banks on their support site:

> <https://sequential.com/support/download/prophet-rev2-sounds/>

Two preset collections are available:

| Download | Contents |
|---|---|
| **Prophet Rev2 Factory Programs** | 4 factory banks (F1–F4) × 128 programs, plus 4 user banks (U1–U4) |
| **Prophet '08 Sound Bank** | 2 factory banks (F5–F6) × 128 programs, originally shipped with the Prophet '08 |

Each download is a `.zip` containing a `.syx` (SysEx) file and a ReadMe with
installation instructions. The factory presets list PDF (showing all program
names) is also available on the page.

## Sending Presets to the Synth-App

The synth-app imports presets via live MIDI SysEx. There is no file-open dialog
— you send the `.syx` file to the app through a virtual MIDI port or loopback
connection.

### Setup

1. Launch the synth-app.
2. Open **Settings** and select a MIDI input port that can receive SysEx data.
3. Ensure the **Patches** toggle for that port is enabled (it is on by default).
4. Use a MIDI utility to send the `.syx` file to the selected port.

### Mac OS — SysEx Librarian

[SysEx Librarian](https://www.snoize.com/SysExLibrarian/) is free software for
sending SysEx files on macOS.

1. Download and install SysEx Librarian.
2. Create a virtual MIDI port (optional): open **Audio MIDI Setup** → Window →
   **Show MIDI Studio** → double-click **IAC Driver** → enable "Device is online".
3. In SysEx Librarian, set the **Destination** to the port you configured in the
   synth-app (e.g. the IAC Bus).
4. Drag the `.syx` file onto the SysEx Librarian window.
5. Click **Play**.

The synth-app will receive each Program Data message, decode it, and save it to
the patches directory.

### Windows — MIDI-OX

[MIDI-OX](http://www.midiox.com/) is free software for sending SysEx files on
Windows.

1. Download and install MIDI-OX.
2. Install a virtual MIDI loopback driver such as
   [loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html) and create
   a port.
3. In MIDI-OX, go to **Options → MIDI Devices** and select the virtual port as
   the output.
4. Go to **View → SysEx**, then **SysEx → Configure**. Set Low Level Output
   Buffers Size to 4096 and disable "Auto-adjust Buffer Delays".
5. From the Command Window menu, choose **Load File** and open the `.syx` file.
6. From the Command Window menu, choose **Send SysEx**.

### Linux

Use `amidi` to send SysEx files. Create a virtual MIDI port with
`snd-virmidi` or use an ALSA loopback device:

```bash
amidi -p <port_name> -s path/to/file.syx
```

## How Imports Work

When the synth-app receives a Program Data SysEx message (`F0 01 2F 02 ... F7`
for Rev2 or `F0 01 23 02 ... F7` for Prophet '08), it:

1. Validates and unpacks the 7-bit packed payload.
2. Decodes Layer A into a patch.
3. Saves the patch as a `.json` file in the patches directory.

Program Edit Buffer messages (`F0 01 2F 03 ... F7` for Rev2, `F0 01 23 03 ... F7`
for Prophet '08) load the patch directly into the active synth engine instead of
saving to disk.

## File Naming

Imported patches are saved with deterministic names based on their bank and
program location:

| Source | Banks | Example Filename |
|---|---|---|
| Rev2 Factory | F1–F4 (banks 0–3) | `F1-001-LosVangelis2041.json` |
| Rev2 User | U1–U4 (banks 4–7) | `U1-052-PolyRadiance.json` |
| Prophet '08 | F5–F6 (banks 0–1) | `F5-001-Wagnerian.json` |

Names use the embedded Layer A name from each program. Receiving the same bank and program
location again overwrites the existing file, making it safe to re-send or re-import.

## Patches Directory

Imported patches are saved alongside user-saved patches in the app's patches
directory:

| OS | Location |
|---|---|
| macOS | `~/Library/Application Support/analog-synth/patches/` |
| Linux | `~/.local/share/analog-synth/patches/` |
| Windows | `C:\Users\<user>\AppData\Roaming\analog-synth\patches\` |

## Notes

- The synth decodes only **Layer A** of each program. Layer B, sequencer,
  arpeggiator, and global settings are ignored. Rev2 and Prophet '08 Glide
  settings are imported; Prophet '08 Glide is enabled when either oscillator
  has a nonzero rate.
- Imported patches do **not** change the active sound — they are saved to disk
  as a library only.
- If the MIDI program import queue fills up, a message is printed to the
  console. Send SysEx files at a reasonable speed (SysEx Librarian and MIDI-OX
  defaults work fine).
- See the [MIDI Implementation](midi-implementation.md) appendix for full
  details on Prophet Rev2 CC/NRPN and SysEx formats, and the Prophet '08
  compatibility section for imported program layout.
