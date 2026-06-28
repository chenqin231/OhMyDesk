import assert from "node:assert/strict";

import {
  containedFrameRect,
  pointerToFrameCoords,
  remoteMouseButtonEvents,
  shouldBlockRemoteContextMenu,
} from "./remote-geometry.ts";

const rect = containedFrameRect(
  { left: 0, top: 0, width: 1600, height: 900 },
  { w: 960, h: 720 },
);

assert.deepEqual(rect, { left: 320, top: 90, width: 960, height: 720 });

assert.equal(
  pointerToFrameCoords({ clientX: 100, clientY: 450 }, rect, { w: 960, h: 720 }),
  null,
);

assert.deepEqual(
  pointerToFrameCoords({ clientX: 800, clientY: 450 }, rect, { w: 960, h: 720 }),
  { x: 480, y: 360 },
);

assert.deepEqual(
  containedFrameRect({ left: 0, top: 0, width: 640, height: 360 }, { w: 1280, h: 720 }),
  { left: 0, top: 0, width: 640, height: 360 },
);

assert.deepEqual(remoteMouseButtonEvents({ x: 480, y: 360 }, 2, true), [
  { kind: "mouse_move", x: 480, y: 360 },
  { kind: "mouse_button", button: 2, down: true },
]);

assert.equal(shouldBlockRemoteContextMenu(), true);
