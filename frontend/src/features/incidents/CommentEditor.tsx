import { useId, type Ref } from "react";
import { CommentMarkdown } from "./CommentMarkdown";

export function CommentEditor({
  value,
  onChange,
  mode,
  onModeChange,
  disabled,
  label,
  textareaRef,
  autoFocus = false,
}: {
  value: string;
  onChange: (value: string) => void;
  mode: "write" | "preview";
  onModeChange: (mode: "write" | "preview") => void;
  disabled: boolean;
  label: string;
  textareaRef?: Ref<HTMLTextAreaElement>;
  autoFocus?: boolean;
}) {
  const helpId = useId();
  return (
    <div className="comment-composer">
      <div className="comment-editor-toolbar">
        <div
          className="comment-editor-modes"
          role="group"
          aria-label={`${label} mode`}
        >
          <button
            type="button"
            aria-pressed={mode === "write"}
            disabled={disabled}
            onClick={() => onModeChange("write")}
          >
            Write
          </button>
          <button
            type="button"
            aria-pressed={mode === "preview"}
            disabled={disabled}
            onClick={() => onModeChange("preview")}
          >
            Preview
          </button>
        </div>
        <span id={helpId}>Markdown supported</span>
      </div>
      {mode === "write" ? (
        <textarea
          ref={textareaRef}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          disabled={disabled}
          aria-label={label}
          aria-describedby={helpId}
          placeholder="What should we know about this incident?"
          autoFocus={autoFocus}
          required
        />
      ) : (
        <div
          className="comment-preview"
          role="region"
          aria-label={`${label} preview`}
        >
          {value.trim() ? (
            <CommentMarkdown body={value} />
          ) : (
            <p className="comment-preview-placeholder">
              Nothing to preview yet.
            </p>
          )}
        </div>
      )}
    </div>
  );
}
