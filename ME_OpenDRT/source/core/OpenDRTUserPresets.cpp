#include "OpenDRTUserPresets.h"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdlib>
#include <ctime>
#if !defined(__linux__)
#include <filesystem>
#endif
#include <fstream>
#include <mutex>
#include <sstream>
#include <string>
#include <unordered_map>

#if defined(__linux__) || defined(__APPLE__)
#include <sys/stat.h>
#endif

namespace MEOpenDRT {
namespace UserPresets {
namespace {

constexpr int kBuiltInLookPresetCount = static_cast<int>(kLookPresetNames.size());
constexpr int kBuiltInTonescalePresetCount = static_cast<int>(kTonescalePresetNames.size());
constexpr int kBuiltInDisplayPresetCount = 9;

UserPresetStore& mutableStore() {
  static UserPresetStore store;
  return store;
}

std::mutex& storeMutex() {
  static std::mutex m;
  return m;
}

std::string nowUtcIso8601() {
  std::time_t t = std::time(nullptr);
  std::tm tm{};
#if defined(_WIN32)
  gmtime_s(&tm, &t);
#else
  gmtime_r(&t, &tm);
#endif
  char buf[32] = {0};
  std::strftime(buf, sizeof(buf), "%Y-%m-%dT%H:%M:%SZ", &tm);
  return std::string(buf);
}

std::string jsonUnescape(const std::string& in) {
  std::string out;
  out.reserve(in.size());
  for (size_t i = 0; i < in.size(); ++i) {
    char c = in[i];
    if (c == '\\' && i + 1 < in.size()) {
      char n = in[++i];
      if (n == 'n') out.push_back('\n');
      else if (n == 'r') out.push_back('\r');
      else if (n == 't') out.push_back('\t');
      else out.push_back(n);
    } else {
      out.push_back(c);
    }
  }
  return out;
}

std::string normalizePresetNameKey(const std::string& s) {
  std::string out;
  out.reserve(s.size());
  bool inSpace = false;
  for (char c : s) {
    const unsigned char uc = static_cast<unsigned char>(c);
    if (std::isspace(uc)) {
      inSpace = true;
      continue;
    }
    if (inSpace && !out.empty()) out.push_back(' ');
    inSpace = false;
    out.push_back(static_cast<char>(std::tolower(uc)));
  }
  while (!out.empty() && out.front() == ' ') out.erase(out.begin());
  while (!out.empty() && out.back() == ' ') out.pop_back();
  return out;
}

std::string sanitizePresetName(const std::string& s, const char* fallback) {
  std::string out;
  out.reserve(s.size());
  for (char c : s) {
    if (c == '\n' || c == '\r' || c == '\t') continue;
    out.push_back(c);
  }
  while (!out.empty() && out.front() == ' ') out.erase(out.begin());
  while (!out.empty() && out.back() == ' ') out.pop_back();
  if (out.empty()) out = fallback;
  if (out.size() > 96) out.resize(96);
  return out;
}

std::string jsonField(const std::string& line, const std::string& key) {
  const std::string token = "\"" + key + "\"";
  const size_t p = line.find(token);
  if (p == std::string::npos) return std::string();
  size_t i = p + token.size();
  while (i < line.size() && std::isspace(static_cast<unsigned char>(line[i]))) ++i;
  if (i >= line.size() || line[i] != ':') return std::string();
  ++i;
  while (i < line.size() && std::isspace(static_cast<unsigned char>(line[i]))) ++i;
  if (i >= line.size() || line[i] != '"') return std::string();
  ++i;
  std::string out;
  bool esc = false;
  for (; i < line.size(); ++i) {
    char c = line[i];
    if (esc) {
      out.push_back('\\');
      out.push_back(c);
      esc = false;
      continue;
    }
    if (c == '\\') {
      esc = true;
      continue;
    }
    if (c == '"') break;
    out.push_back(c);
  }
  return jsonUnescape(out);
}

std::string jsonObjectField(const std::string& text, const std::string& key) {
  const std::string token = "\"" + key + "\":";
  const size_t p = text.find(token);
  if (p == std::string::npos) return std::string();
  size_t i = p + token.size();
  while (i < text.size() && std::isspace(static_cast<unsigned char>(text[i]))) ++i;
  if (i >= text.size() || text[i] != '{') return std::string();
  const size_t start = i;
  int depth = 0;
  bool inString = false;
  bool esc = false;
  for (; i < text.size(); ++i) {
    const char c = text[i];
    if (inString) {
      if (esc) {
        esc = false;
      } else if (c == '\\') {
        esc = true;
      } else if (c == '"') {
        inString = false;
      }
      continue;
    }
    if (c == '"') {
      inString = true;
      continue;
    }
    if (c == '{') {
      ++depth;
    } else if (c == '}') {
      --depth;
      if (depth == 0) return text.substr(start, i - start + 1);
    }
  }
  return std::string();
}

bool extractJsonObjectFromStream(std::istream& is, const std::string& firstLine, std::string* outObj) {
  if (!outObj) return false;
  std::string obj = firstLine;
  int depth = 0;
  bool inString = false;
  bool esc = false;
  auto scan = [&](const std::string& s) {
    for (char c : s) {
      if (inString) {
        if (esc) {
          esc = false;
        } else if (c == '\\') {
          esc = true;
        } else if (c == '"') {
          inString = false;
        }
        continue;
      }
      if (c == '"') {
        inString = true;
      } else if (c == '{') {
        ++depth;
      } else if (c == '}') {
        --depth;
      }
    }
  };
  scan(firstLine);
  while (depth > 0 && is.good()) {
    std::string next;
    if (!std::getline(is, next)) break;
    obj.append("\n");
    obj.append(next);
    scan(next);
  }
  if (depth != 0) return false;
  *outObj = obj;
  return true;
}

bool jsonNumberField(const std::string& obj, const char* key, double* out) {
  if (!out || !key) return false;
  const std::string token = std::string("\"") + key + "\":";
  const size_t p = obj.find(token);
  if (p == std::string::npos) return false;
  size_t i = p + token.size();
  while (i < obj.size() && std::isspace(static_cast<unsigned char>(obj[i]))) ++i;
  if (i >= obj.size()) return false;
  char* endp = nullptr;
  const double v = std::strtod(obj.c_str() + i, &endp);
  if (endp == obj.c_str() + i) return false;
  *out = v;
  return true;
}

template <typename T>
bool jsonNumberFieldAs(const std::string& obj, const char* key, T* out) {
  if (!out) return false;
  double v = 0.0;
  if (!jsonNumberField(obj, key, &v)) return false;
  *out = static_cast<T>(v);
  return true;
}

bool parseLookValues(const std::string& in, LookPresetValues* v) {
  if (!v) return false;
  std::istringstream is(in);
  return static_cast<bool>(
      is >> v->tn_con >> v->tn_sh >> v->tn_toe >> v->tn_off
         >> v->tn_hcon_enable >> v->tn_hcon >> v->tn_hcon_pv >> v->tn_hcon_st
         >> v->tn_lcon_enable >> v->tn_lcon >> v->tn_lcon_w
         >> v->cwp >> v->cwp_lm
         >> v->rs_sa >> v->rs_rw >> v->rs_bw
         >> v->pt_enable
         >> v->pt_lml >> v->pt_lml_r >> v->pt_lml_g >> v->pt_lml_b
         >> v->pt_lmh >> v->pt_lmh_r >> v->pt_lmh_b
         >> v->ptl_enable >> v->ptl_c >> v->ptl_m >> v->ptl_y
         >> v->ptm_enable >> v->ptm_low >> v->ptm_low_rng >> v->ptm_low_st
         >> v->ptm_high >> v->ptm_high_rng >> v->ptm_high_st
         >> v->brl_enable >> v->brl >> v->brl_r >> v->brl_g >> v->brl_b
         >> v->brl_rng >> v->brl_st
         >> v->brlp_enable >> v->brlp >> v->brlp_r >> v->brlp_g >> v->brlp_b
         >> v->hc_enable >> v->hc_r >> v->hc_r_rng
         >> v->hs_rgb_enable >> v->hs_r >> v->hs_r_rng
         >> v->hs_g >> v->hs_g_rng >> v->hs_b >> v->hs_b_rng
         >> v->hs_cmy_enable >> v->hs_c >> v->hs_c_rng >> v->hs_m >> v->hs_m_rng
         >> v->hs_y >> v->hs_y_rng);
}

bool parseTonescaleValues(const std::string& in, TonescalePresetValues* v) {
  if (!v) return false;
  std::istringstream is(in);
  return static_cast<bool>(
      is >> v->tn_con >> v->tn_sh >> v->tn_toe >> v->tn_off
         >> v->tn_hcon_enable >> v->tn_hcon >> v->tn_hcon_pv >> v->tn_hcon_st
         >> v->tn_lcon_enable >> v->tn_lcon >> v->tn_lcon_w);
}

bool parseLookValuesFromNamedJson(const std::string& obj, LookPresetValues* v) {
  if (!v || obj.empty()) return false;
  auto reqD = [&](const char* k, auto* d) -> bool { return jsonNumberFieldAs(obj, k, d); };
  auto reqI = [&](const char* k, int* d) -> bool {
    double x = 0.0;
    if (!jsonNumberField(obj, k, &x)) return false;
    *d = static_cast<int>(std::llround(x));
    return true;
  };
  return reqD("tn_con", &v->tn_con) && reqD("tn_sh", &v->tn_sh) && reqD("tn_toe", &v->tn_toe) && reqD("tn_off", &v->tn_off) &&
         reqI("tn_hcon_enable", &v->tn_hcon_enable) && reqD("tn_hcon", &v->tn_hcon) && reqD("tn_hcon_pv", &v->tn_hcon_pv) && reqD("tn_hcon_st", &v->tn_hcon_st) &&
         reqI("tn_lcon_enable", &v->tn_lcon_enable) && reqD("tn_lcon", &v->tn_lcon) && reqD("tn_lcon_w", &v->tn_lcon_w) &&
         reqI("cwp", &v->cwp) && reqD("cwp_lm", &v->cwp_lm) &&
         reqD("rs_sa", &v->rs_sa) && reqD("rs_rw", &v->rs_rw) && reqD("rs_bw", &v->rs_bw) &&
         reqI("pt_enable", &v->pt_enable) &&
         reqD("pt_lml", &v->pt_lml) && reqD("pt_lml_r", &v->pt_lml_r) && reqD("pt_lml_g", &v->pt_lml_g) && reqD("pt_lml_b", &v->pt_lml_b) &&
         reqD("pt_lmh", &v->pt_lmh) && reqD("pt_lmh_r", &v->pt_lmh_r) && reqD("pt_lmh_b", &v->pt_lmh_b) &&
         reqI("ptl_enable", &v->ptl_enable) && reqD("ptl_c", &v->ptl_c) && reqD("ptl_m", &v->ptl_m) && reqD("ptl_y", &v->ptl_y) &&
         reqI("ptm_enable", &v->ptm_enable) && reqD("ptm_low", &v->ptm_low) && reqD("ptm_low_rng", &v->ptm_low_rng) && reqD("ptm_low_st", &v->ptm_low_st) &&
         reqD("ptm_high", &v->ptm_high) && reqD("ptm_high_rng", &v->ptm_high_rng) && reqD("ptm_high_st", &v->ptm_high_st) &&
         reqI("brl_enable", &v->brl_enable) && reqD("brl", &v->brl) && reqD("brl_r", &v->brl_r) && reqD("brl_g", &v->brl_g) && reqD("brl_b", &v->brl_b) &&
         reqD("brl_rng", &v->brl_rng) && reqD("brl_st", &v->brl_st) &&
         reqI("brlp_enable", &v->brlp_enable) && reqD("brlp", &v->brlp) && reqD("brlp_r", &v->brlp_r) && reqD("brlp_g", &v->brlp_g) && reqD("brlp_b", &v->brlp_b) &&
         reqI("hc_enable", &v->hc_enable) && reqD("hc_r", &v->hc_r) && reqD("hc_r_rng", &v->hc_r_rng) &&
         reqI("hs_rgb_enable", &v->hs_rgb_enable) && reqD("hs_r", &v->hs_r) && reqD("hs_r_rng", &v->hs_r_rng) &&
         reqD("hs_g", &v->hs_g) && reqD("hs_g_rng", &v->hs_g_rng) && reqD("hs_b", &v->hs_b) && reqD("hs_b_rng", &v->hs_b_rng) &&
         reqI("hs_cmy_enable", &v->hs_cmy_enable) && reqD("hs_c", &v->hs_c) && reqD("hs_c_rng", &v->hs_c_rng) &&
         reqD("hs_m", &v->hs_m) && reqD("hs_m_rng", &v->hs_m_rng) && reqD("hs_y", &v->hs_y) && reqD("hs_y_rng", &v->hs_y_rng);
}

bool parseTonescaleValuesFromNamedJson(const std::string& obj, TonescalePresetValues* v) {
  if (!v || obj.empty()) return false;
  auto reqD = [&](const char* k, auto* d) -> bool { return jsonNumberFieldAs(obj, k, d); };
  auto reqI = [&](const char* k, int* d) -> bool {
    double x = 0.0;
    if (!jsonNumberField(obj, k, &x)) return false;
    *d = static_cast<int>(std::llround(x));
    return true;
  };
  return reqD("tn_con", &v->tn_con) && reqD("tn_sh", &v->tn_sh) && reqD("tn_toe", &v->tn_toe) && reqD("tn_off", &v->tn_off) &&
         reqI("tn_hcon_enable", &v->tn_hcon_enable) && reqD("tn_hcon", &v->tn_hcon) && reqD("tn_hcon_pv", &v->tn_hcon_pv) &&
         reqD("tn_hcon_st", &v->tn_hcon_st) && reqI("tn_lcon_enable", &v->tn_lcon_enable) && reqD("tn_lcon", &v->tn_lcon) &&
         reqD("tn_lcon_w", &v->tn_lcon_w);
}

bool parseStartupDefaultSettingsFromJson(const std::string& obj, StartupDefaultSettings* out) {
  if (!out || obj.empty()) return false;
  StartupDefaultSettings s = *out;
  double d = 0.0;
  if (jsonNumberField(obj, "inGamut", &d)) s.inGamut = static_cast<int>(std::llround(d));
  if (jsonNumberField(obj, "inOetf", &d)) s.inOetf = static_cast<int>(std::llround(d));
  if (jsonNumberField(obj, "displayEncodingPreset", &d)) s.displayEncodingPreset = static_cast<int>(std::llround(d));
  if (jsonNumberField(obj, "greyLuminance", &d)) s.greyLuminance = static_cast<float>(d);
  if (jsonNumberField(obj, "lookPreset", &d)) s.lookPreset = static_cast<int>(std::llround(d));
  if (jsonNumberField(obj, "tonescalePreset", &d)) s.tonescalePreset = static_cast<int>(std::llround(d));
  if (jsonNumberField(obj, "creativeWhitePreset", &d)) s.creativeWhitePreset = static_cast<int>(std::llround(d));
  if (jsonNumberField(obj, "creativeWhiteLimit", &d)) s.creativeWhiteLimit = static_cast<float>(d);
  *out = clampStartupDefaultSettingsBasic(s);
  return true;
}

void ensureLoadedLocked() {
  UserPresetStore& s = mutableStore();
  if (s.loaded) return;
  s = UserPresetStore{};
  s.loaded = true;
  s.startupDefaults = factoryStartupDefaultSettings();

  std::ifstream file(userPresetFilePathV2(), std::ios::binary);
  if (!file.is_open()) return;

  std::string content((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
  if (content.empty()) return;

  const std::string startupDefaultsJson = jsonObjectField(content, "startupDefaults");
  if (!startupDefaultsJson.empty()) {
    parseStartupDefaultSettingsFromJson(startupDefaultsJson, &s.startupDefaults);
  }

  enum class Section { None, Look, Tone };
  Section sec = Section::None;
  std::unordered_map<std::string, bool> seenLookNames;
  std::unordered_map<std::string, bool> seenToneNames;
  for (const char* n : kLookPresetNames) seenLookNames[normalizePresetNameKey(n)] = true;
  for (const char* n : kTonescalePresetNames) seenToneNames[normalizePresetNameKey(n)] = true;

  std::istringstream is(content);
  std::string line;
  while (std::getline(is, line)) {
    if (line.find("\"lookPresets\"") != std::string::npos) {
      sec = Section::Look;
      continue;
    }
    if (line.find("\"tonescalePresets\"") != std::string::npos) {
      sec = Section::Tone;
      continue;
    }
    if (sec == Section::None) continue;
    if (line.find(']') != std::string::npos) {
      sec = Section::None;
      continue;
    }
    if (line.find('{') == std::string::npos) continue;

    std::string obj;
    if (!extractJsonObjectFromStream(is, line, &obj)) continue;
    if (obj.find("\"id\"") == std::string::npos) continue;

    const std::string id = jsonField(obj, "id");
    const std::string name = sanitizePresetName(jsonField(obj, "name"), "User Preset");
    const std::string created = jsonField(obj, "createdAtUtc");
    const std::string updated = jsonField(obj, "updatedAtUtc");
    const std::string payload = jsonField(obj, "payload");
    const std::string namedValues = jsonObjectField(obj, "namedValues");
    if (id.empty()) continue;

    if (sec == Section::Look) {
      const std::string key = normalizePresetNameKey(name);
      if (seenLookNames.find(key) != seenLookNames.end()) continue;
      LookPresetValues parsed{};
      bool parsedOk = false;
      if (!payload.empty()) parsedOk = parseLookValues(payload, &parsed);
      if (!parsedOk && !namedValues.empty()) parsedOk = parseLookValuesFromNamedJson(namedValues, &parsed);
      if (!parsedOk) continue;
      UserLookPreset p{};
      p.id = id;
      p.name = name;
      p.createdAtUtc = created.empty() ? nowUtcIso8601() : created;
      p.updatedAtUtc = updated.empty() ? p.createdAtUtc : updated;
      p.values = parsed;
      s.lookPresets.push_back(p);
      seenLookNames[key] = true;
    } else if (sec == Section::Tone) {
      const std::string key = normalizePresetNameKey(name);
      if (seenToneNames.find(key) != seenToneNames.end()) continue;
      TonescalePresetValues parsed{};
      bool parsedOk = false;
      if (!payload.empty()) parsedOk = parseTonescaleValues(payload, &parsed);
      if (!parsedOk && !namedValues.empty()) parsedOk = parseTonescaleValuesFromNamedJson(namedValues, &parsed);
      if (!parsedOk) continue;
      UserTonescalePreset p{};
      p.id = id;
      p.name = name;
      p.createdAtUtc = created.empty() ? nowUtcIso8601() : created;
      p.updatedAtUtc = updated.empty() ? p.createdAtUtc : updated;
      p.values = parsed;
      s.tonescalePresets.push_back(p);
      seenToneNames[key] = true;
    }
  }
  s.startupDefaults = clampStartupDefaultSettingsForStore(s.startupDefaults, s);
}

}  // namespace

std::string userPresetDirPath() {
#if defined(_WIN32)
  const char* base = std::getenv("APPDATA");
  if (!base || !*base) base = std::getenv("LOCALAPPDATA");
  if (base && *base) return (std::filesystem::path(base) / "ME_OpenDRT").string();
  return ".";
#elif defined(__APPLE__)
  const char* home = std::getenv("HOME");
  if (home && *home) return (std::filesystem::path(home) / "Library" / "Application Support" / "ME_OpenDRT").string();
  return ".";
#else
  const char* home = std::getenv("HOME");
  if (home && *home) return std::string(home) + "/.config/ME_OpenDRT";
  return ".";
#endif
}

std::string userPresetFilePathV2() {
#if defined(_WIN32) || defined(__APPLE__)
  return (std::filesystem::path(userPresetDirPath()) / "presets_v2.json").string();
#else
  return userPresetDirPath() + "/presets_v2.json";
#endif
}

StartupDefaultSettings factoryStartupDefaultSettings() {
  return StartupDefaultSettings{};
}

StartupDefaultSettings clampStartupDefaultSettingsBasic(StartupDefaultSettings s) {
  s.inGamut = std::clamp(s.inGamut, 0, 14);
  s.inOetf = std::clamp(s.inOetf, 0, 9);
  s.displayEncodingPreset = std::clamp(s.displayEncodingPreset, 0, kBuiltInDisplayPresetCount - 1);
  s.greyLuminance = std::clamp(s.greyLuminance, 3.0f, 25.0f);
  s.lookPreset = std::max(0, s.lookPreset);
  s.tonescalePreset = std::max(0, s.tonescalePreset);
  s.creativeWhitePreset = std::clamp(s.creativeWhitePreset, 0, 6);
  s.creativeWhiteLimit = std::clamp(s.creativeWhiteLimit, 0.0f, 1.0f);
  return s;
}

StartupDefaultSettings clampStartupDefaultSettingsForStore(StartupDefaultSettings s,
                                                           const UserPresetStore& store) {
  s = clampStartupDefaultSettingsBasic(s);
  const int maxLook = kBuiltInLookPresetCount + static_cast<int>(store.lookPresets.size()) - 1;
  const int maxTone = kBuiltInTonescalePresetCount + static_cast<int>(store.tonescalePresets.size()) - 1;
  s.lookPreset = (s.lookPreset > maxLook) ? 0 : s.lookPreset;
  s.tonescalePreset = (s.tonescalePreset > maxTone) ? 0 : s.tonescalePreset;
  return s;
}

const UserPresetStore& currentUserPresetStore() {
  std::lock_guard<std::mutex> lock(storeMutex());
  ensureLoadedLocked();
  return mutableStore();
}

void reloadUserPresetStoreFromDisk() {
  std::lock_guard<std::mutex> lock(storeMutex());
  mutableStore() = UserPresetStore{};
  ensureLoadedLocked();
}

std::vector<std::string> visibleUserLookNames() {
  std::lock_guard<std::mutex> lock(storeMutex());
  ensureLoadedLocked();
  std::vector<std::string> out;
  for (const auto& p : mutableStore().lookPresets) out.push_back(p.name);
  return out;
}

std::vector<std::string> visibleUserTonescaleNames() {
  std::lock_guard<std::mutex> lock(storeMutex());
  ensureLoadedLocked();
  std::vector<std::string> out;
  for (const auto& p : mutableStore().tonescalePresets) out.push_back(p.name);
  return out;
}

bool applyLookPresetIndexToResolved(OpenDRTParams& p, int presetIndex) {
  if (presetIndex < kBuiltInLookPresetCount) {
    applyLookPresetToResolved(p, presetIndex);
    return presetIndex >= 0;
  }
  std::lock_guard<std::mutex> lock(storeMutex());
  ensureLoadedLocked();
  const int rel = presetIndex - kBuiltInLookPresetCount;
  if (rel < 0 || rel >= static_cast<int>(mutableStore().lookPresets.size())) {
    applyLookPresetToResolved(p, 0);
    return false;
  }
  const LookPresetValues& s = mutableStore().lookPresets[static_cast<size_t>(rel)].values;
  p.tn_con = s.tn_con; p.tn_sh = s.tn_sh; p.tn_toe = s.tn_toe; p.tn_off = s.tn_off;
  p.tn_hcon_enable = s.tn_hcon_enable; p.tn_hcon = s.tn_hcon; p.tn_hcon_pv = s.tn_hcon_pv; p.tn_hcon_st = s.tn_hcon_st;
  p.tn_lcon_enable = s.tn_lcon_enable; p.tn_lcon = s.tn_lcon; p.tn_lcon_w = s.tn_lcon_w;
  p.cwp = s.cwp; p.cwp_lm = s.cwp_lm;
  p.rs_sa = s.rs_sa; p.rs_rw = s.rs_rw; p.rs_bw = s.rs_bw;
  p.pt_enable = s.pt_enable; p.pt_lml = s.pt_lml; p.pt_lml_r = s.pt_lml_r; p.pt_lml_g = s.pt_lml_g; p.pt_lml_b = s.pt_lml_b;
  p.pt_lmh = s.pt_lmh; p.pt_lmh_r = s.pt_lmh_r; p.pt_lmh_b = s.pt_lmh_b;
  p.ptl_enable = s.ptl_enable; p.ptl_c = s.ptl_c; p.ptl_m = s.ptl_m; p.ptl_y = s.ptl_y;
  p.ptm_enable = s.ptm_enable; p.ptm_low = s.ptm_low; p.ptm_low_rng = s.ptm_low_rng; p.ptm_low_st = s.ptm_low_st;
  p.ptm_high = s.ptm_high; p.ptm_high_rng = s.ptm_high_rng; p.ptm_high_st = s.ptm_high_st;
  p.brl_enable = s.brl_enable; p.brl = s.brl; p.brl_r = s.brl_r; p.brl_g = s.brl_g; p.brl_b = s.brl_b; p.brl_rng = s.brl_rng; p.brl_st = s.brl_st;
  p.brlp_enable = s.brlp_enable; p.brlp = s.brlp; p.brlp_r = s.brlp_r; p.brlp_g = s.brlp_g; p.brlp_b = s.brlp_b;
  p.hc_enable = s.hc_enable; p.hc_r = s.hc_r; p.hc_r_rng = s.hc_r_rng;
  p.hs_rgb_enable = s.hs_rgb_enable; p.hs_r = s.hs_r; p.hs_r_rng = s.hs_r_rng; p.hs_g = s.hs_g; p.hs_g_rng = s.hs_g_rng; p.hs_b = s.hs_b; p.hs_b_rng = s.hs_b_rng;
  p.hs_cmy_enable = s.hs_cmy_enable; p.hs_c = s.hs_c; p.hs_c_rng = s.hs_c_rng; p.hs_m = s.hs_m; p.hs_m_rng = s.hs_m_rng; p.hs_y = s.hs_y; p.hs_y_rng = s.hs_y_rng;
  return true;
}

bool applyTonescalePresetIndexToResolved(OpenDRTParams& p, int presetIndex) {
  if (presetIndex <= 0) return true;
  if (presetIndex < kBuiltInTonescalePresetCount) {
    applyTonescalePresetToResolved(p, presetIndex);
    return true;
  }
  std::lock_guard<std::mutex> lock(storeMutex());
  ensureLoadedLocked();
  const int rel = presetIndex - kBuiltInTonescalePresetCount;
  if (rel < 0 || rel >= static_cast<int>(mutableStore().tonescalePresets.size())) return false;
  const TonescalePresetValues& t = mutableStore().tonescalePresets[static_cast<size_t>(rel)].values;
  p.tn_con = t.tn_con; p.tn_sh = t.tn_sh; p.tn_toe = t.tn_toe; p.tn_off = t.tn_off;
  p.tn_hcon_enable = t.tn_hcon_enable; p.tn_hcon = t.tn_hcon; p.tn_hcon_pv = t.tn_hcon_pv; p.tn_hcon_st = t.tn_hcon_st;
  p.tn_lcon_enable = t.tn_lcon_enable; p.tn_lcon = t.tn_lcon; p.tn_lcon_w = t.tn_lcon_w;
  return true;
}

StartupDefaultSettings describeStartupDefaultSettings() {
  std::lock_guard<std::mutex> lock(storeMutex());
  ensureLoadedLocked();
  mutableStore().startupDefaults = clampStartupDefaultSettingsForStore(mutableStore().startupDefaults, mutableStore());
  return mutableStore().startupDefaults;
}

OpenDRTParams resolveStartupDefaultSettings(StartupDefaultSettings settings,
                                            StartupDefaultSettings* resolvedSettings,
                                            int* activeLookSlot,
                                            int* activeToneSlot) {
  OpenDRTParams p{};
  int lookSlot = -1;
  int toneSlot = -1;
  {
    std::lock_guard<std::mutex> lock(storeMutex());
    ensureLoadedLocked();
    settings = clampStartupDefaultSettingsForStore(settings, mutableStore());
    if (settings.lookPreset >= kBuiltInLookPresetCount) {
      lookSlot = settings.lookPreset - kBuiltInLookPresetCount;
    }
    if (settings.tonescalePreset >= kBuiltInTonescalePresetCount) {
      toneSlot = settings.tonescalePreset - kBuiltInTonescalePresetCount;
    }
  }

  applyLookPresetIndexToResolved(p, settings.lookPreset);
  applyTonescalePresetIndexToResolved(p, settings.tonescalePreset);
  applyDisplayEncodingPreset(p, settings.displayEncodingPreset);
  p.in_gamut = settings.inGamut;
  p.in_oetf = settings.inOetf;
  p.tn_Lp = (p.eotf == 4 || p.eotf == 5) ? 1000.0f : 100.0f;
  p.tn_gb = 0.13f;
  p.pt_hdr = 0.5f;
  p.tn_Lg = settings.greyLuminance;
  p.crv_enable = 0;
  p.clamp = 1;
  p.cwp_lm = settings.creativeWhiteLimit;
  if (settings.creativeWhitePreset > 0) {
    p.cwp = settings.creativeWhitePreset - 1;
  }
  if (resolvedSettings) *resolvedSettings = settings;
  if (activeLookSlot) *activeLookSlot = lookSlot;
  if (activeToneSlot) *activeToneSlot = toneSlot;
  return p;
}

}  // namespace UserPresets
}  // namespace MEOpenDRT
