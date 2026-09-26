# Frontend style guide

The frontend follows the TypeScript conventions from Google's TypeScript
style guide and the React/JSX conventions from Airbnb's React style guide.
Prettier and ESLint enforce the mechanical parts of those conventions.

## Formatting

- Run `npm run format` before committing.
- Run `npm run format:check` in CI.
- Use two spaces, semicolons, double quotes, trailing commas, and an 80-column
  print width.
- Keep one blank line between imports, top-level declarations, functions, and
  logical sections inside a component.
- Keep at most one consecutive blank line.

## Components

- Prefer functional components and hooks.
- Keep one primary component per file when practical.
- Split a component when it owns multiple independent workflows or becomes
  difficult to scan. Extract panels, tables, and repeated forms into focused
  components before adding more conditional JSX.
- Keep data loading, event handlers, and rendered sections visually grouped.
- Keep JSX props on separate lines when an element does not fit comfortably on
  one line.

## TypeScript and React

- Prefer `const`; use `let` only when reassignment is required.
- Use `import type` for type-only imports.
- Avoid `any`, non-null assertions, ignored type errors, and nested ternaries.
- Follow the Rules of Hooks and keep components pure.
- Use descriptive names for exported components, functions, and data types.
