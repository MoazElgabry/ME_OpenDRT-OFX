#pragma once

#include <string>
#include <vector>

#include "OpenDRTPresets.h"

namespace MEOpenDRT {
namespace UserPresets {

struct UserLookPreset {
  std::string id;
  std::string name;
  std::string createdAtUtc;
  std::string updatedAtUtc;
  LookPresetValues values{};
};

struct UserTonescalePreset {
  std::string id;
  std::string name;
  std::string createdAtUtc;
  std::string updatedAtUtc;
  TonescalePresetValues values{};
};

struct StartupDefaultSettings {
  int inGamut = 14;
  int inOetf = 1;
  int displayEncodingPreset = 0;
  float greyLuminance = 10.0f;
  int lookPreset = 0;
  int tonescalePreset = 0;
  int creativeWhitePreset = 0;
  float creativeWhiteLimit = 0.25f;
};

struct UserPresetStore {
  bool loaded = false;
  StartupDefaultSettings startupDefaults;
  std::vector<UserLookPreset> lookPresets;
  std::vector<UserTonescalePreset> tonescalePresets;
};

std::string userPresetDirPath();
std::string userPresetFilePathV2();

StartupDefaultSettings factoryStartupDefaultSettings();
StartupDefaultSettings clampStartupDefaultSettingsBasic(StartupDefaultSettings s);
StartupDefaultSettings clampStartupDefaultSettingsForStore(StartupDefaultSettings s,
                                                           const UserPresetStore& store);

const UserPresetStore& currentUserPresetStore();
void reloadUserPresetStoreFromDisk();

std::vector<std::string> visibleUserLookNames();
std::vector<std::string> visibleUserTonescaleNames();

bool applyLookPresetIndexToResolved(OpenDRTParams& p, int presetIndex);
bool applyTonescalePresetIndexToResolved(OpenDRTParams& p, int presetIndex);

StartupDefaultSettings describeStartupDefaultSettings();
OpenDRTParams resolveStartupDefaultSettings(StartupDefaultSettings settings,
                                            StartupDefaultSettings* resolvedSettings = nullptr,
                                            int* activeLookSlot = nullptr,
                                            int* activeToneSlot = nullptr);

}  // namespace UserPresets
}  // namespace MEOpenDRT
