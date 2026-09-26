import js from "@eslint/js";
import prettier from "eslint-config-prettier";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: [
      "**/node_modules/",
      "**/dist/",
      "**/dist-electron/",
      "**/out/",
      "**/release/",
      "packages/protocol/src/generated/",
      "packages/mobile/ios/",
      "packages/mobile/android/",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  ...tseslint.configs.stylisticTypeChecked,
  {
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      "@typescript-eslint/consistent-type-imports": ["error", { fixStyle: "inline-type-imports" }],
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
    },
  },
  {
    files: ["packages/app/**/*.{ts,tsx}"],
    languageOptions: { globals: globals.browser },
    plugins: { "react-hooks": reactHooks, "react-refresh": reactRefresh },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
    },
  },
  {
    files: ["packages/protocol/**/*.ts"],
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
  },
  {
    files: [
      "packages/desktop/**/*.ts",
      "packages/mobile/**/*.ts",
      "**/*.config.{js,ts}",
      "packages/protocol/scripts/**/*.ts",
    ],
    languageOptions: { globals: globals.node },
  },
  {
    files: ["**/*.js", "**/*.config.ts"],
    ...tseslint.configs.disableTypeChecked,
  },
  prettier,
);
