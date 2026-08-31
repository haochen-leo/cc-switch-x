import { getIdentifier } from "@tauri-apps/api/app";

export const OFFICIAL_APP_IDENTIFIER = "com.ccswitch.desktop";
export const CC_SWITCH_X_APP_IDENTIFIER = "io.github.haochen-leo.ccswitchx";

let officialUpdateSupportPromise: Promise<boolean> | null = null;

export function supportsOfficialInAppUpdate(): Promise<boolean> {
  if (!officialUpdateSupportPromise) {
    officialUpdateSupportPromise = getIdentifier()
      .then((identifier) => identifier === OFFICIAL_APP_IDENTIFIER)
      .catch(() => false);
  }

  return officialUpdateSupportPromise;
}
