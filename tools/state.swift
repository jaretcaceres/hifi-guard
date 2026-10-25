// Prints default devices and every device's transport/rate. Run: swift tools/state.swift
import CoreAudio
func addr(_ s: AudioObjectPropertySelector, _ sc: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress { .init(mSelector: s, mScope: sc, mElement: kAudioObjectPropertyElementMain) }
func u32(_ id: AudioObjectID, _ s: AudioObjectPropertySelector) -> UInt32 { var a = addr(s); var v: UInt32 = 0; var z = UInt32(4); AudioObjectGetPropertyData(id, &a, 0, nil, &z, &v); return v }
func rate(_ id: AudioObjectID) -> Double { var a = addr(kAudioDevicePropertyNominalSampleRate); var v = 0.0; var z = UInt32(8); AudioObjectGetPropertyData(id, &a, 0, nil, &z, &v); return v }
func name(_ id: AudioObjectID) -> String { var a = addr(kAudioObjectPropertyName); var v: Unmanaged<CFString>?; var z = UInt32(8); AudioObjectGetPropertyData(id, &a, 0, nil, &z, &v); return (v?.takeRetainedValue() as String?) ?? "?" }
let sys = AudioObjectID(kAudioObjectSystemObject)
func t(_ id: AudioObjectID) -> String { let x = u32(id, kAudioDevicePropertyTransportType); return x == kAudioDeviceTransportTypeBluetooth ? "BT" : x == kAudioDeviceTransportTypeUSB ? "USB" : x == kAudioDeviceTransportTypeBuiltIn ? "Built-in" : "other" }
for (label, sel) in [("OUT", kAudioHardwarePropertyDefaultOutputDevice), ("IN ", kAudioHardwarePropertyDefaultInputDevice), ("SYS", kAudioHardwarePropertyDefaultSystemOutputDevice)] {
  let id = u32(sys, sel); print("\(label): \(name(id)) [\(t(id))] \(Int(rate(id))) Hz")
}
var a = addr(kAudioHardwarePropertyDevices); var z: UInt32 = 0; AudioObjectGetPropertyDataSize(sys, &a, 0, nil, &z)
var ids = [AudioObjectID](repeating: 0, count: Int(z)/4); AudioObjectGetPropertyData(sys, &a, 0, nil, &z, &ids)
for id in ids where t(id) == "BT" { print("  BT device: \(name(id)) \(Int(rate(id))) Hz") }
