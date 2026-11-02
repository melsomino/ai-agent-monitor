// Run with: gjs -m tests/ubuntu_panel_geometry.js
import {popupPosition} from '../src/linux/gnome-extension/geometry.js';

function check(anchor, frame, area, expected) {
    const actual = popupPosition(anchor, frame, area);
    if (JSON.stringify(actual) !== JSON.stringify(expected))
        throw new Error(`Expected ${expected}, got ${actual}`);
}
check({x: 1000, y: 0, width: 24, height: 32}, {width: 392, height: 476},
    {x: 0, y: 32, width: 1920, height: 1048}, [816, 32]);
check({x: 1890, y: 0, width: 24, height: 32}, {width: 392, height: 476},
    {x: 0, y: 32, width: 1920, height: 1048}, [1524, 32]);
check({x: -1910, y: 0, width: 24, height: 32}, {width: 392, height: 476},
    {x: -1920, y: 32, width: 1920, height: 1048}, [-1916, 32]);
check({x: 1000, y: 0, width: 24, height: 32}, {width: 392, height: 1000},
    {x: 0, y: 32, width: 1920, height: 768}, [816, 32]);
print('Panel anchoring and monitor-edge geometry checks passed');
