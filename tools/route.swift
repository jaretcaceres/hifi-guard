// Sets default output/input to the Bluetooth A50, like macOS does on reconnect. Run: swift tools/route.swift
import CoreAudio
func addr(_ s: AudioObjectPropertySelector, _ sc: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress { .init(mSelector: s, mScope: sc, mElement: kAudioObjectPropertyElementMain) }
let sys = AudioObjectID(kAudioObjectSystemObject)
func streams(_ id: AudioObjectID, _ sc: AudioObjectPropertyScope) -> Bool { var a = addr(kAudioDevicePropertyStreams, sc); var z: UInt32 = 0; AudioObjectGetPropertyDataSize(id, &a, 0, nil, &z); return z > 0 }
func bt(_ id: AudioObjectID) -> Bool { var a = addr(kAudioDevicePropertyTransportType); var v: UInt32 = 0; var z = UInt32(4); AudioObjectGetPropertyData(id, &a, 0, nil, &z, &v); return v == kAudioDeviceTransportTypeBluetooth }
var a = addr(kAudioHardwarePropertyDevices); var z: UInt32 = 0; AudioObjectGetPropertyDataSize(sys, &a, 0, nil, &z)
var ids = [AudioObjectID](repeating: 0, count: Int(z)/4); AudioObjectGetPropertyData(sys, &a, 0, nil, &z, &ids)
for var id in ids where bt(id) {
  for (sel, sc) in [(kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeOutput), (kAudioHardwarePropertyDefaultSystemOutputDevice, kAudioObjectPropertyScopeOutput), (kAudioHardwarePropertyDefaultInputDevice, kAudioObjectPropertyScopeInput)] where streams(id, sc) {
    var s = addr(sel); AudioObjectSetPropertyData(sys, &s, 0, nil, 4, &id)
  }
}
