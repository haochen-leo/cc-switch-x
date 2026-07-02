import { getIdentifier } from "@tauri-apps/api/app";

export const OFFICIAL_APP_IDENTIFIER = "com.ccswitch.desktop";
export const IDEALAB_APP_IDENTIFIER = "com.ccswitch.idealab";

let officialUpdateSupportPromise: Promise<boolean> | null = null;

export function supportsOfficialInAppUpdate(): Promise<boolean> {
  if (!officialUpdateSupportPromise) {
    officialUpdateSupportPromise = getIdentifier()
      .then((identifier) => identifier === OFFICIAL_APP_IDENTIFIER)
      .catch(() => true);
  }

  return officialUpdateSupportPromise;
}
