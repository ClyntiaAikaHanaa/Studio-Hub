/// <reference types="svelte" />
/// <reference types="vite/client" />

// Vite mengubah impor aset menjadi URL string saat build. Tanpa deklarasi ini,
// TypeScript tidak tahu bentuk hasilnya dan `import logo from "./x.png"`
// dianggap modul yang tidak ada.
declare module "*.png" {
  const src: string;
  export default src;
}

declare module "*.svg" {
  const src: string;
  export default src;
}

declare module "*.webp" {
  const src: string;
  export default src;
}
