import { useId } from "react";
import Markdown from "react-markdown";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import rehypeSlug from "rehype-slug";
import rehypeKatex from "rehype-katex";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import remarkEmoji from "remark-emoji";
import {
  remarkDefinitionList,
  defListHastHandlers,
} from "remark-definition-list";
import type { Root, Element } from "hast";
import { MermaidDiagram } from "./MermaidDiagram";
import "katex/dist/katex.min.css";
import "highlight.js/styles/github.css";

// Sanitization prefixes every ID. Apply the same prefix to local references,
// including footnotes, so multiple updates/comments cannot share targets.
function localReferences(prefix: string) {
  return (tree: Root) => {
    function visit(node: Root | Element) {
      if (node.type === "element") {
        const href = node.properties.href;
        if (typeof href === "string" && href.startsWith("#")) {
          node.properties.href = `#${prefix}${href.slice(1)}`;
        }
        for (const key of ["ariaDescribedBy", "ariaLabelledBy"]) {
          const ids = node.properties[key];
          if (Array.isArray(ids))
            node.properties[key] = ids.map((id) => `${prefix}${id}`);
        }
      }
      for (const child of node.children)
        if (child.type === "element") visit(child);
    }

    visit(tree);
  };
}

export function MarkdownContent({ value }: { value: string }) {
  const id = useId();
  const prefix = `markdown-${id.replace(/[^a-zA-Z0-9_-]/g, "")}-`;
  const schema = {
    ...defaultSchema,
    clobberPrefix: prefix,
    attributes: {
      ...defaultSchema.attributes,
      code: [["className", /^language-./, "math-inline", "math-display"]],
      // Preserve image descriptions without allowing Markdown to initiate requests.
      img: ["alt", "title"],
    },
    strip: [
      ...(defaultSchema.strip ?? []),
      "style",
      "iframe",
      "object",
      "embed",
      "svg",
      "math",
      "form",
    ],
  };
  return (
    <div className="markdown-content">
      <Markdown
        remarkPlugins={[
          remarkGfm,
          remarkMath,
          remarkEmoji,
          remarkDefinitionList,
        ]}
        remarkRehypeOptions={{
          clobberPrefix: "",
          handlers: defListHastHandlers,
        }}
        rehypePlugins={[
          rehypeRaw,
          rehypeSlug,
          [rehypeSanitize, schema],
          [localReferences, prefix],
          // Only trusted renderers run after sanitizing the source HTML.
          [rehypeKatex, { trust: false, maxExpand: 1000, maxSize: 20 }],
          [
            rehypeHighlight,
            { detect: false, ignoreMissing: true, plainText: ["mermaid"] },
          ],
        ]}
        components={{
          a: ({
            href,
            title,
            id,
            children,
            "aria-describedby": describedBy,
            "aria-label": label,
          }) =>
            href ? (
              <a
                href={href}
                title={title}
                id={id}
                aria-describedby={describedBy}
                aria-label={label}
                target={href.startsWith("#") ? undefined : "_blank"}
                rel="noreferrer noopener"
              >
                {children}
              </a>
            ) : (
              <span id={id}>{children}</span>
            ),
          img: ({ alt }) => <span>{alt}</span>,
          pre: ({ node, children }) => {
            const code = node?.children[0];
            if (
              code?.type === "element" &&
              code.tagName === "code" &&
              Array.isArray(code.properties.className) &&
              code.properties.className.includes("language-mermaid")
            ) {
              const source = code.children
                .filter((child) => child.type === "text")
                .map((child) => child.value)
                .join("");
              return <MermaidDiagram source={source} />;
            }
            return <pre>{children}</pre>;
          },
          table: ({ children }) => (
            <div className="comment-table-scroll">
              <table>{children}</table>
            </div>
          ),
        }}
      >
        {value}
      </Markdown>
    </div>
  );
}
