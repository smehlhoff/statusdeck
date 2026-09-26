import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { ApiFailure, api } from "../../api/client";
import { queryKeys } from "../../api/queries";
import type { DisplayPreferences, User } from "../../api/types";
import { useToast } from "../../components/toastContext";
import { useProfile } from "./profileContext";

const THEMES = [
  {
    value: "light",
    label: "Light",
    detail: "A bright background with dark text.",
  },
  {
    value: "dark",
    label: "Dark",
    detail: "A dark background with light text.",
  },
  {
    value: "system",
    label: "System",
    detail: "Follow your device’s appearance.",
  },
] as const;

export function Appearance() {
  const profile = useProfile();
  const [theme, setTheme] = useState<DisplayPreferences["theme"]>(
    profile.preferences.theme,
  );
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const save = useMutation({
    mutationFn: () =>
      api<User>("/api/v1/profile", {
        method: "PATCH",
        body: JSON.stringify({ preferences: { theme } }),
      }),
    onSuccess: async (updated) => {
      await queryClient.cancelQueries({ queryKey: queryKeys.session });
      queryClient.setQueryData(queryKeys.session, updated);
      notify("Appearance saved.");
    },
  });

  return (
    <section
      className="card profile-section"
      aria-labelledby="profile-appearance"
    >
      <header className="profile-section-heading">
        <h2 id="profile-appearance">Appearance</h2>
        <p className="profile-help">
          Choose how StatusDeck looks across the app.
        </p>
      </header>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (!save.isPending) save.mutate();
        }}
      >
        <fieldset disabled={save.isPending}>
          <legend className="sr-only">Color theme</legend>
          <div className="appearance-options">
            {THEMES.map((option) => (
              <label className="appearance-option" key={option.value}>
                <span
                  className={`appearance-preview appearance-preview-${option.value}`}
                  aria-hidden="true"
                >
                  <span />
                  <span />
                </span>
                <span className="appearance-choice">
                  <input
                    type="radio"
                    name="theme"
                    value={option.value}
                    checked={theme === option.value}
                    onChange={() => setTheme(option.value)}
                  />
                  {option.label}
                </span>
                <span className="profile-help">{option.detail}</span>
              </label>
            ))}
          </div>
        </fieldset>
        {save.isError && (
          <p className="alert error" role="alert">
            {save.error instanceof ApiFailure
              ? save.error.message
              : "Appearance could not be saved. Try again."}
          </p>
        )}
        <button
          className="button primary"
          disabled={save.isPending || theme === profile.preferences.theme}
        >
          {save.isPending ? "Saving…" : "Save appearance"}
        </button>
      </form>
    </section>
  );
}
