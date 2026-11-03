# README screenshots

The five macOS PNGs were extracted from the privacy-redacted final demo video, `final.mp4`. Cropping excludes the video captions; no UI elements were reconstructed or added. The exported zooms and masks are preserved. Images retain their source pixel resolution within each crop.

| File | Source timestamp | Crop (width × height, x, y) | State |
|---|---|---|---|
| `sessions.png` | 00:01:34.500 | 860 × 900, 800, 0 | Four working sessions, expanded monitor, VS Code in the background |
| `questions.png` | 00:00:27.000 | 670 × 662, 908, 288 | Two questions, radio choices, custom inputs, shared submit |
| `approvals.png` | 00:01:25.500 | 780 × 660, 764, 180 | Approval with command context and all three actions |
| `notifications.png` | 00:01:19.500 | 1410 × 780, 248, 142 | Native notification action menu; Telegram notification is also visible behind it |
| `telegram.png` | 00:01:28.500 | 674 × 966, 780, 62 | Session list and approval response buttons in the bot chat |

Timestamps refer to the final 97-second edit, not the original screen recording. Crops use source-frame coordinates from the 1670 × 1080 MP4.

Example extraction:

```sh
ffmpeg -ss 00:00:27.000 -i final.mp4 -frames:v 1 \
  -vf 'crop=670:662:908:288' -map_metadata -1 questions.png
```

`ubuntu.png` is a separately supplied 2550 × 1520 screenshot of the running Ubuntu GNOME frontend, with VS Code in the background. It is not a frame from the macOS demo video.
