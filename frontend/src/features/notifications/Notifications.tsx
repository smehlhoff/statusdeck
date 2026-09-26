import { useCallback, useState } from "react";
import { useBeforeUnload, useBlocker } from "react-router-dom";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { AlertRulePanel } from "./AlertRulePanel";
import { ChannelPanel } from "./ChannelPanel";

export function Notifications() {
  const [channelFormDirty, setChannelFormDirty] = useState(false);
  const [ruleFormDirty, setRuleFormDirty] = useState(false);
  const [highlightedChannelId, setHighlightedChannelId] = useState<string>();
  const hasUnsavedChanges = channelFormDirty || ruleFormDirty;
  const blocker = useBlocker(hasUnsavedChanges);

  useBeforeUnload(
    useCallback(
      (event) => {
        if (!hasUnsavedChanges) return;
        event.preventDefault();
        event.returnValue = "";
      },
      [hasUnsavedChanges],
    ),
  );

  function highlightRules(channelId: string) {
    setHighlightedChannelId((current) =>
      current === channelId ? undefined : channelId,
    );
    requestAnimationFrame(() => {
      document
        .getElementById("configured-alert-rules")
        ?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    });
  }

  return (
    <>
      <div className="page-heading">
        <div>
          <h1>Notifications</h1>
          <p className="muted">
            Configure destinations and decide which monitored events should
            notify them.
          </p>
        </div>
      </div>

      <div className="notification-layout">
        <ChannelPanel
          highlightedChannelId={highlightedChannelId}
          onDirtyChange={setChannelFormDirty}
          onHighlightRules={highlightRules}
        />
        <AlertRulePanel
          highlightedChannelId={highlightedChannelId}
          onDirtyChange={setRuleFormDirty}
        />
      </div>
      {blocker.state === "blocked" && (
        <ConfirmDialog
          title="Discard unsaved changes?"
          message="Your alert-rule or delivery-channel changes have not been saved."
          confirmLabel="Discard changes"
          onConfirm={blocker.proceed}
          onClose={blocker.reset}
        />
      )}
    </>
  );
}
