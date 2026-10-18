import js from "@eslint/js";
import globals from "globals";

export default [
  {
    files: ["web/js/**/*.js"],
    languageOptions: {
      sourceType: "module",
      globals: globals.browser,
    },
    rules: js.configs.recommended.rules,
  },
];
