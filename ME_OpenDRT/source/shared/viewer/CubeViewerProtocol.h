#pragma once

#include <string_view>

namespace MEOpenDRT {
namespace CubeViewerProtocol {

enum class SourceMode {
  IdentityCube = 0,
  InputImage = 1
};

enum class Quality {
  Low = 0,
  Medium = 1,
  High = 2
};

struct ViewerParamsState {
  SourceMode sourceMode = SourceMode::IdentityCube;
  Quality quality = Quality::Medium;
  int resolution = 41;
  bool plotInLinear = false;
  bool showOverflow = true;
  bool highlightOverflow = true;
  bool alwaysOnTop = true;
};

constexpr std::string_view kViewerExecutableBaseName = "ME_OpenDRT_CubeViewer";
constexpr std::string_view kViewerTransportBaseName = "ME_OpenDRT_CubeViewer";

inline constexpr int resolutionForQuality(Quality quality) {
  switch (quality) {
    case Quality::Low:
      return 25;
    case Quality::High:
      return 57;
    case Quality::Medium:
    default:
      return 41;
  }
}

inline constexpr const char* qualityLabel(Quality quality) {
  switch (quality) {
    case Quality::Low:
      return "Low";
    case Quality::High:
      return "High";
    case Quality::Medium:
    default:
      return "Medium";
  }
}

inline constexpr const char* sourceModeLabel(SourceMode sourceMode) {
  switch (sourceMode) {
    case SourceMode::InputImage:
      return "input";
    case SourceMode::IdentityCube:
    default:
      return "identity";
  }
}

}  // namespace CubeViewerProtocol
}  // namespace MEOpenDRT
