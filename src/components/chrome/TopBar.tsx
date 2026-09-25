/**
 * The navigation bar on every screen but the dashboard (upstream `PopoverTopBar`): a back button, the
 * centered title and the screen's trailing actions (Customize: undo and reset).
 */
import { messagesFor } from "@/i18n";
import { resetProvider } from "@/model/layout";
import { providerTitle } from "@/model/providerText";
import { useLanguage } from "@/state/hooks";
import { navigate, openCustomizeDetail, resetAllCustomization, undoLayout, updateLayout, useApp, type Screen } from "@/state/store";
import { confirmAction } from "../ui/dialog";
import { ChevronLeft, ResetIcon, UndoIcon } from "../ui/icons";
import { tooltipProps } from "../ui/tooltip";

export function goBack(): void {
  const { screen, customizeProviderId } = useApp.getState();
  if (screen === "customize" && customizeProviderId) openCustomizeDetail(null);
  else navigate("dashboard");
}

export function TopBar({ screen }: { screen: Screen }) {
  const language = useLanguage();
  const messages = messagesFor(language);
  const catalog = useApp((state) => state.catalog);
  const providerId = useApp((state) => state.customizeProviderId);
  const canUndo = useApp((state) => state.undoStack.length > 0);
  const provider = providerId ? catalog.find((entry) => entry.provider.id === providerId)?.provider : undefined;

  let title = messages.chrome.settings;
  if (screen === "customize") title = provider ? providerTitle(provider, language) : messages.chrome.customize;
  if (screen === "accounts") title = messages.chrome.accounts;

  const resetAll = async () => {
    const confirmed = await confirmAction({
      title: messages.chrome.resetAllTitle,
      message: messages.chrome.resetAllMessage,
      confirmLabel: messages.chrome.resetAllConfirm,
      cancelLabel: messages.chrome.cancel,
    });
    if (confirmed) resetAllCustomization();
  };

  return (
    <header className="uc-topbar">
      <button type="button" className="uc-circle-button" aria-label={messages.chrome.back} onClick={goBack} {...tooltipProps(messages.chrome.back)}>
        <ChevronLeft size={12} />
      </button>
      <h1 className="uc-topbar-title uc-truncate">{title}</h1>
      <div className="uc-topbar-trailing">
        {screen === "customize" ? (
          <>
            <button type="button" className="uc-circle-button" aria-label={messages.customize.undo} disabled={!canUndo} onClick={() => undoLayout()} {...tooltipProps(messages.customize.undo)}>
              <UndoIcon size={12} />
            </button>
            {provider ? (
              <button
                type="button"
                className="uc-circle-button"
                aria-label={messages.chrome.resetProvider(title)}
                onClick={() => updateLayout((layout) => resetProvider(layout, catalog, provider.id))}
                {...tooltipProps(messages.chrome.resetProvider(title))}
              >
                <ResetIcon size={12} />
              </button>
            ) : (
              <button type="button" className="uc-circle-button" aria-label={messages.chrome.resetAll} onClick={() => void resetAll()} {...tooltipProps(messages.chrome.resetAll)}>
                <ResetIcon size={12} />
              </button>
            )}
          </>
        ) : null}
      </div>
    </header>
  );
}
