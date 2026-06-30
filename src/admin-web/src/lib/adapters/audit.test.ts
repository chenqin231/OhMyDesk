import { describe, it, expect } from "vitest";
import { summarize, type TimelineItem } from "./audit";

function item(kind: TimelineItem["kind"], text = ""): TimelineItem {
  return { ts: 0, kind, text };
}

describe("summarize", () => {
  it("空 → 无操作记录", () => {
    expect(summarize([])).toBe("无操作记录");
  });

  it("统计截图次数", () => {
    expect(summarize([item("screenshot"), item("screenshot")])).toBe("截图 2 次");
  });

  it("input 用其 text 直拼", () => {
    expect(summarize([item("input", "输入操作 47 次")])).toBe("输入操作 47 次");
  });

  it("识别 command/file_transfer/chat 并计数", () => {
    const items = [
      item("command"),
      item("command"),
      item("file_transfer"),
      item("chat"),
      item("chat"),
      item("chat"),
    ];
    expect(summarize(items)).toBe("命令 2 条，文件传输 1 次，消息 3 条");
  });

  it("混合类型按固定顺序拼接", () => {
    const items = [
      item("screenshot"),
      item("input", "输入操作 5 次"),
      item("command"),
      item("file_transfer"),
      item("chat"),
    ];
    expect(summarize(items)).toBe("截图 1 次，输入操作 5 次，命令 1 条，文件传输 1 次，消息 1 条");
  });
});
