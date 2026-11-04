import { getIdentifier } from "@tauri-apps/api/app";

export const OFFICIAL_APP_IDENTIFIER = "com.ccswitch.desktop";
export const CC_SWITCH_X_APP_IDENTIFIER = "io.github.haochen-leo.ccswitchx";

let updateSupportPromise: Promise<boolean> | null = null;

export function supportsInAppUpdate(): Promise<boolean> {
  if (!updateSupportPromise) {
    updateSupportPromise = getIdentifier()
      // Each build uses its own configured endpoint and signing public key.
      .then(
        (identifier) =>
          identifier === OFFICIAL_APP_IDENTIFIER ||
          identifier === CC_SWITCH_X_APP_IDENTIFIER,
      )
      .catch(() => false);
  }

  return updateSupportPromise;
}
