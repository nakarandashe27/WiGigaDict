export type RuntimeProfileOption = {
  id: string;
  modelName: string;
  modelVersion: string;
  deviceKind: string;
  healthState: string;
  available: boolean;
};

export type CleanupProfileOption = {
  id: string;
  name: string;
  profileVersion: number;
};

export type InputDeviceStatus = {
  id: string;
  name: string;
  isDefault: boolean;
  healthy: boolean;
};

export type AppConfiguration = {
  configVersion: number;
  hotkeyBinding: string;
  microphoneDeviceId: string | null;
  activeRuntimeProfileId: string | null;
  activeCleanupProfileId: string | null;
  startupEnabled: boolean;
  warmupEnabled: boolean;
  diagnosticMode: boolean;
  archiveDirectory: string;
};

export type SettingsView = {
  configuration: AppConfiguration;
  runtimeProfiles: RuntimeProfileOption[];
  cleanupProfiles: CleanupProfileOption[];
  inputDevices: InputDeviceStatus[];
  inputDeviceError: string | null;
  startupRegistered: boolean;
};

export type ConfigurationUpdate = {
  expectedConfigVersion: number;
  hotkeyBinding: string;
  microphoneDeviceId: string | null;
  activeRuntimeProfileId: string | null;
  activeCleanupProfileId: string | null;
  startupEnabled: boolean;
  warmupEnabled: boolean;
  diagnosticMode: boolean;
  archiveDirectory: string;
};

export function configurationUpdate(
  configuration: AppConfiguration,
): ConfigurationUpdate {
  return {
    expectedConfigVersion: configuration.configVersion,
    hotkeyBinding: configuration.hotkeyBinding,
    microphoneDeviceId: configuration.microphoneDeviceId,
    activeRuntimeProfileId: configuration.activeRuntimeProfileId,
    activeCleanupProfileId: configuration.activeCleanupProfileId,
    startupEnabled: configuration.startupEnabled,
    warmupEnabled: configuration.warmupEnabled,
    diagnosticMode: configuration.diagnosticMode,
    archiveDirectory: configuration.archiveDirectory,
  };
}

export function validateConfiguration(
  update: ConfigurationUpdate,
): string | null {
  const hotkey = update.hotkeyBinding.trim();
  if (hotkey.length === 0 || hotkey.length > 64) {
    return "Укажите сочетание длиной до 64 символов.";
  }
  if (!/[+]/u.test(hotkey) && !/^F(?:[1-9]|1[0-2])$/iu.test(hotkey)) {
    return "Без модификатора можно использовать только F1–F12.";
  }
  if (update.warmupEnabled && !update.activeRuntimeProfileId) {
    return "Для прогрева сначала выберите установленную модель.";
  }
  const archive = update.archiveDirectory.trim();
  if (archive.length < 3 || archive.length > 1024) {
    return "Укажите папку архива длиной до 1024 символов.";
  }
  if (!/^[a-z]:[\\/]/iu.test(archive)) {
    return "Архив должен находиться в локальной папке с буквой диска.";
  }
  return null;
}

export function runtimeLabel(profile: RuntimeProfileOption): string {
  const device =
    profile.deviceKind === "vulkan"
      ? "GPU"
      : profile.deviceKind === "cpu"
        ? "CPU"
        : profile.deviceKind;
  return `${profile.modelName} ${profile.modelVersion} · ${device}`;
}
