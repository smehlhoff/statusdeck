import type * as MermaidModule from "mermaid";
import { useEffect, useId, useState } from "react";

const MAX_DIAGRAM_LENGTH = 10_000;
let mermaidModule: Promise<typeof MermaidModule> | undefined;

function loadMermaid() {
  mermaidModule ??= import("mermaid").then((module) => {
    module.default.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      suppressErrorRendering: true,
      maxTextSize: MAX_DIAGRAM_LENGTH,
      maxEdges: 200,
      htmlLabels: false,
      flowchart: { htmlLabels: false },
      secure: [
        "secure",
        "securityLevel",
        "startOnLoad",
        "maxTextSize",
        "maxEdges",
        "suppressErrorRendering",
        "htmlLabels",
        "flowchart",
      ],
    });
    return module;
  });
  return mermaidModule;
}

export function MermaidDiagram({ source }: { source: string }) {
  const id = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const [result, setResult] = useState<{ source: string; url?: string }>({
    source: "",
  });
  useEffect(() => {
    let cancelled = false;
    let url: string | undefined;

    async function render() {
      try {
        if (source.length > MAX_DIAGRAM_LENGTH)
          throw new Error("Diagram too large");
        const { default: mermaid } = await loadMermaid();
        if (cancelled) return;
        const { svg } = await mermaid.render(`diagram-${id}`, source);
        if (cancelled) return;
        // SVG images cannot run scripts or affect the application's DOM/styles.
        url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
        setResult({ source, url });
      } catch {
        if (!cancelled) setResult({ source });
      }
    }

    void render();
    return () => {
      cancelled = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [id, source]);
  const current = result.source === source;
  return (
    <div className="markdown-diagram">
      {current && result.url ? (
        <img src={result.url} alt="Mermaid diagram" />
      ) : (
        <p role="status">
          {current
            ? "Diagram could not be rendered. Source is shown below."
            : "Rendering diagram…"}
        </p>
      )}
      <details open={current && !result.url}>
        <summary>Diagram source</summary>
        <pre>
          <code>{source}</code>
        </pre>
      </details>
    </div>
  );
}
