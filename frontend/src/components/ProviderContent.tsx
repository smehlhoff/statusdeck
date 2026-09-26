import { MarkdownContent } from "./MarkdownContent";

export function ProviderContent({ value }: { value: string }) {
  return (
    <div className="provider-message">
      <MarkdownContent value={value} />
    </div>
  );
}
