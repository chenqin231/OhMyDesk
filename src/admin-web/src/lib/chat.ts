// 会话内即时消息的纯逻辑：消息条目类型 + 追加 reducer。
// 抽成纯函数便于单测；store 持有 chatMessages 数组并调用这里的 appendChat。

// 一条会话消息。mine=true 表示本端（管理员）发出，渲染靠右；false 为对端（被控方）。
export type ChatEntry = {
  msg_id: string;
  text: string;
  mine: boolean;
  ts: number; // 毫秒级本地时间，仅用于展示/排序
};

// 追加一条消息；按 msg_id 去重（自己乐观追加后，server 不回显自己的消息；
// 但若链路回环或重发，去重避免重复气泡）。返回新数组（不可变更新）。
export function appendChat(list: ChatEntry[], entry: ChatEntry): ChatEntry[] {
  if (list.some((m) => m.msg_id === entry.msg_id)) return list;
  return [...list, entry];
}
