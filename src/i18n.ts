import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";

import zh from "./locales/zh.json";
import en from "./locales/en.json";

// 首启（无存储值）必须传 undefined：i18next 显式传 lng 会整体短路
// LanguageDetector，navigator 探测不执行——英文系统的全新安装会永远进
// 中文界面。有存储值时直接用，检测器没必要跑。
const saved = localStorage.getItem("ds-lang") ?? undefined;

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: { zh: { translation: zh }, en: { translation: en } },
    lng: saved,
    fallbackLng: "zh",
    interpolation: { escapeValue: false },
    detection: {
      order: ["localStorage", "navigator"],
      caches: ["localStorage"],
      lookupLocalStorage: "ds-lang",
    },
  });

export default i18n;
