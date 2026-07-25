# Factory Presets

This page explains how to load Sequential's official Prophet Rev2 factory
sound banks into hardware program memory over MIDI. For the desktop
development harness, see [Application: Development Harness](../application.md).

## Downloading the Presets

Sequential publishes the factory sound banks on their support site:

> <https://sequential.com/support/download/prophet-rev2-sounds/>

| Download | Contents |
|---|---|
| **Prophet Rev2 Factory Programs** | 4 factory banks (F1–F4) × 128 programs, plus 4 user banks (U1–U4) |

Each download is a `.zip` containing a `.syx` (SysEx) file and a ReadMe. The
factory presets list PDF (showing all program names) is also available on the
page.

Hardware program storage accepts **Prophet Rev2** Program Data only
(`F0 01 2F 02 … F7`). Prophet '08 SysEx is not written to program memory; use
the [desktop harness](../application.md) if you need those banks.

## Sending Presets

1. Connect the synthesizer so its MIDI input is available to the host.
2. Open a SysEx utility and set the destination to that MIDI port.
3. Send the Rev2 `.syx` file. Each Program Data message saves Layer A into the
   bank and program encoded in the message (banks 0–7, programs 0–127).
   Firmware applies USB backpressure while flash writes complete, so a full
   512-program dump can take on the order of a minute; wait for the transfer
   to finish before recalling.
4. Recall with Bank Select (CC0 or CC32, value 0–7) followed by Program Change.

Program Edit Buffer messages (`F0 01 2F 03 … F7`) load the live patch only and
do not write program memory.

Send at a moderate rate so the device can accept each dump (SysEx Librarian
and MIDI-OX defaults are fine). Do not power-cycle mid-transfer: a full bank
set writes many flash sectors.

### Mac OS — SysEx Librarian

[SysEx Librarian](https://www.snoize.com/SysExLibrarian/) is free software for
sending SysEx files on macOS.

1. Download and install SysEx Librarian.
2. Confirm the synthesizer's MIDI port appears in Audio MIDI Setup.
3. In SysEx Librarian, set the **Destination** to that port.
4. Drag the `.syx` file onto the SysEx Librarian window.
5. Click **Play**.

### Windows — MIDI-OX

[MIDI-OX](http://www.midiox.com/) is free software for sending SysEx files on
Windows.

1. Download and install MIDI-OX.
2. Note the synthesizer's MIDI port name.
3. In MIDI-OX, go to **Options → MIDI Devices** and select that port as the
   output.
4. Go to **View → SysEx**, then **SysEx → Configure**. Set Low Level Output
   Buffers Size to 4096 and disable "Auto-adjust Buffer Delays".
5. From the Command Window menu, choose **Load File** and open the `.syx` file.
6. From the Command Window menu, choose **Send SysEx**.

### Linux

Use `amidi` to send SysEx to the synthesizer's ALSA MIDI port:

```bash
amidi -p <port_name> -s path/to/file.syx
```

List ports with `amidi -l`.

## What Gets Stored

- Only **Layer A** is stored. Layer B, sequencer, arpeggiator, and global
  settings are ignored. Glide settings from the Rev2 image are imported.
- There is no factory/user bank split on the device: banks 0–7 are ordinary
  persistent slots. Rev2 factory files typically use banks 0–3 (F1–F4) and
  4–7 (U1–U4).
- Empty or freshly formatted slots recall the default patch until overwritten.
- See the [MIDI Spec](midi-spec.md) for Program Data framing, Bank Select /
  Program Change behavior, and the Rev2 program image layout.
