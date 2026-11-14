# README screenshots

The video-derived macOS PNGs were extracted from the privacy-redacted final demo video, `final.mp4`. Cropping excludes the video captions; no UI elements were reconstructed or added. The approval screenshot is a fresh capture of the production AppKit renderer using synthetic regression fixtures, not a frame from the recording. No image generation or UI reconstruction was used.

| File | Source timestamp | Crop (width × height, x, y) | State |
|---|---|---|---|
| `sessions.png` | 00:01:34.500 | 860 × 900, 800, 0 | Four working sessions, expanded monitor, VS Code in the background |
| `questions.png` | 00:00:27.000 | 670 × 662, 908, 288 | Two questions, radio choices, custom inputs, shared submit |
| `approvals.png` | AppKit fixture capture | 920 × 1508, full renderer viewport | Synthetic Task Board approval with command context, all three actions and expanded command scope |
| `notifications.png` | 00:01:19.500 | 1410 × 780, 248, 142 | Native notification action menu; Telegram notification is also visible behind it |
| `telegram.png` | 00:01:31.000 | 674 × 966, 780, 62 | Demo agent messages and session list, without internal build paths |

Timestamps refer to the final 97-second edit, not the original screen recording. Crops use source-frame coordinates from the 1670 × 1080 MP4.

Example extraction:

```sh
ffmpeg -ss 00:00:27.000 -i final.mp4 -frames:v 1 \
  -vf 'crop=670:662:908:288' -map_metadata -1 questions.png
```

`ubuntu.png` is a separately supplied 2550 × 1520 screenshot of the running Ubuntu GNOME frontend, with VS Code in the background. It is not a frame from the macOS demo video.

To regenerate the native approval capture on macOS without reading live sessions or showing a window:

```sh
AAM_VISUAL_PREVIEW_DIR=/tmp/aam-preview cargo test --offline --test visual_frontend
cp /tmp/aam-preview/dark-scope-expanded.png docs/images/approvals.png
```
