#pragma once

#include <string>

#include "OpenDRTParams.h"

namespace OpenDRTLut {

enum class FormatProfile {
  GenericCube = 0,
  SonyCameraCube = 1,
  ArriColorToolCube = 2
};

struct ExportOptions {
  FormatProfile profile = FormatProfile::GenericCube;
  int resolution = 33;
  std::string title;
  std::string comment;
};

struct ExportResult {
  bool ok = false;
  std::string error;
};

bool isSupportedResolution(FormatProfile profile, int resolution);
int defaultResolution(FormatProfile profile);
const char* profileLabel(FormatProfile profile);
ExportResult writeCubeFile(const std::string& path, const OpenDRTParams& params, const ExportOptions& options);

}  // namespace OpenDRTLut
