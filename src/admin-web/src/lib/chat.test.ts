import { describe, it, expect } from "vitest";
import { appendChat, type ChatEntry } from "./chat";

function entry(msg_id: string, text: string, mine: boolean): ChatEntry {
  return { msg_id, text, mine, ts: 0 };
}

describe("appendChat", () => {
  it("空列表追加一条", () => {
    const out = appendChat([], entry("m1", "你好", true));
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ msg_id: "m1", text: "你好", mine: true });
  });

  it("保持时间正序追加在末尾", () => {
    let list: ChatEntry[] = [];
    list = appendChat(list, entry("m1", "first", true));
    list = appendChat(list, entry("m2", "second", false));
    expect(list.map((m) => m.msg_id)).toEqual(["m1", "m2"]);
  });

  it("按 msg_id 去重，不重复追加", () => {
    let list = appendChat([], entry("m1", "你好", true));
    list = appendChat(list, entry("m1", "你好", true));
    expect(list).toHaveLength(1);
  });

  it("返回新数组（不可变）", () => {
    const before: ChatEntry[] = [];
    const after = appendChat(before, entry("m1", "x", true));
    expect(after).not.toBe(before);
    expect(before).toHaveLength(0);
  });
});
