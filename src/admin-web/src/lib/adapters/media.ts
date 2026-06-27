// 截图/帧 data URI 适配（消化数据源差异）
export const screenshotSrc = (r: { data: string }): string =>
  `data:image/jpeg;base64,${r.data}`;

export const frameSrc = (f: { data: string }): string =>
  `data:image/jpeg;base64,${f.data}`;
