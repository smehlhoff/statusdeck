import { MarkdownContent } from "../../components/MarkdownContent";

export function CommentMarkdown({ body }: { body: string }) {
  return (
    <div className="comment-body comment-markdown">
      <MarkdownContent value={body} />
    </div>
  );
}
