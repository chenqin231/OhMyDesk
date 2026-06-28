import { useEffect } from "react";
import { useStore } from "@/store";

// 在 App 级别初始化 Transport，只执行一次
export function useTransport() {
  const initTransport = useStore((s) => s.initTransport);

  useEffect(() => {
    initTransport();
    return () => {
      // disconnect 在 transport/index.ts 的 transport.disconnect() 中处理
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
