cask "hifi-guard" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "https://github.com/jaretcaceres/hifi-guard/releases/download/v#{version}/HiFi-Guard-#{version}.zip"
  name "HiFi Guard"
  desc "Keeps Bluetooth headsets out of call mode"
  homepage "https://github.com/jaretcaceres/hifi-guard"

  depends_on macos: ">= :sonoma"

  app "HiFi Guard.app"

  uninstall launchctl: "com.jaret.hifiguard",
            quit:      "com.jaret.hifiguard"

  zap trash: [
    "~/Library/Application Support/HiFiGuard",
    "~/Library/LaunchAgents/com.jaret.hifiguard.plist",
  ]
end
