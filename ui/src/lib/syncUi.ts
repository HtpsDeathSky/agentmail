export function getManualSyncButtonState(selectedAccountId: string | null, isManualSyncing: boolean) {
  return {
    className: isManualSyncing ? "icon-button sync-button syncing" : "icon-button sync-button",
    disabled: !selectedAccountId || isManualSyncing,
    title: isManualSyncing ? "Sync running" : "Sync account"
  };
}
