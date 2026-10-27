// Type-aware lint focused on async safety: no promise may be left without a
// handler, so failure paths can never surface as unhandled rejections.
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist/", "example/", "node_modules/", "smoke/"] },
  {
    files: ["src/**/*.ts", "test/**/*.ts"],
    extends: [tseslint.configs.base],
    languageOptions: {
      parserOptions: { projectService: { allowDefaultProject: ["test/*.ts"] } },
    },
    rules: {
      "@typescript-eslint/no-floating-promises": "error",
      "@typescript-eslint/no-misused-promises": "error",
      "@typescript-eslint/return-await": ["error", "in-try-catch"],
    },
  },
);
