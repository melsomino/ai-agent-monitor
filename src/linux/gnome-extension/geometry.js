// Coordinates are compositor logical pixels, including fractional-scale setups.
export function popupPosition(anchor, frame, area) {
    const left = Math.round(Math.max(area.x + 4, Math.min(
        anchor.x + anchor.width / 2 - frame.width / 2,
        area.x + area.width - frame.width - 4)));
    const top = Math.round(Math.max(area.y, Math.min(
        anchor.y + anchor.height, area.y + area.height - frame.height - 4)));
    return [left, top];
}
