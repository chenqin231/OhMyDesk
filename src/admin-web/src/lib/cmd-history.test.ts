import { describe, it, expect } from "vitest";
import { navPrev, navNext, pushHistory } from "./cmd-history";

describe("cmd-history", () => {
  const h = ["ls", "pwd", "whoami"]; // 最新在末尾

  it("↑ 初次从最新一条开始", () => {
    expect(navPrev(h, null)).toEqual({ cursor: 2, text: "whoami" });
  });

  it("连续 ↑ 往旧走，封底在最旧", () => {
    expect(navPrev(h, 2)).toEqual({ cursor: 1, text: "pwd" });
    expect(navPrev(h, 1)).toEqual({ cursor: 0, text: "ls" });
    expect(navPrev(h, 0)).toEqual({ cursor: 0, text: "ls" }); // 封底
  });

  it("空历史 ↑ 无操作", () => {
    expect(navPrev([], null)).toEqual({ cursor: null, text: null });
  });

  it("↓ 往新走，越过最新回到空白当前输入", () => {
    expect(navNext(h, 0)).toEqual({ cursor: 1, text: "pwd" });
    expect(navNext(h, 2)).toEqual({ cursor: null, text: "" });
  });

  it("↓ 在 null（未浏览）时无操作", () => {
    expect(navNext(h, null)).toEqual({ cursor: null, text: null });
  });

  it("pushHistory 追加在末尾、去连续重复、忽略空白", () => {
    expect(pushHistory(h, "ls -al")).toEqual(["ls", "pwd", "whoami", "ls -al"]);
    expect(pushHistory(h, "whoami")).toEqual(h); // 与上一条重复
    expect(pushHistory(h, "   ")).toEqual(h); // 空白忽略
  });
});
