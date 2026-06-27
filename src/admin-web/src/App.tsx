import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { useEffect } from "react";
import { Assets } from "@/pages/Assets";
import { Grid } from "@/pages/Grid";
import { Remote } from "@/pages/Remote";
import { Audit } from "@/pages/Audit";
import { Assistant } from "@/pages/Assistant";
import { useStore } from "@/store";

// App 级别初始化 Transport（VITE_USE_MOCK=1 使用 mock，否则连真实 server）
function TransportInit() {
  const initTransport = useStore((s) => s.initTransport);
  useEffect(() => {
    initTransport();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return null;
}

export function App() {
  return (
    <BrowserRouter>
      <TransportInit />
      <Routes>
        <Route path="/" element={<Navigate to="/assets" replace />} />
        <Route path="/assets" element={<Assets />} />
        <Route path="/grid" element={<Grid />} />
        <Route path="/remote" element={<Remote />} />
        <Route path="/audit" element={<Audit />} />
        <Route path="/assistant" element={<Assistant />} />
      </Routes>
    </BrowserRouter>
  );
}
