// Mixamo .fbx -> VRM Humanoid retarget + gesture catalog.
//
// The Gestures Pack Basic (Adobe Mixamo) ships 15 short upper-body
// animations on a `mixamorig:*` skeleton. This module loads one on
// demand, rewrites the bone names to match the target VRM's
// normalized humanoid, and returns a Three.js AnimationClip ready
// to feed into a single shared AnimationMixer. The clip is cached
// per (url, vrm) so switching back and forth between gestures
// doesn't re-fetch the .fbx.
//
// Position tracks other than the hip are dropped — Mixamo's
// skeleton proportions differ from any given VRM, and translating
// a Mixamo elbow keyframe onto a different upper-arm length
// drifts the character. Quaternion (rotation) tracks are what
// actually communicate the gesture and they retarget cleanly.

import * as THREE from "three";
import { FBXLoader } from "three/examples/jsm/loaders/FBXLoader.js";
import { type VRM, VRMHumanBoneName } from "@pixiv/three-vrm";
import type { Emotion } from "../voice/emotion";

const MIXAMO_TO_VRM: Record<string, VRMHumanBoneName> = {
  "mixamorig:Hips": VRMHumanBoneName.Hips,
  "mixamorig:Spine": VRMHumanBoneName.Spine,
  "mixamorig:Spine1": VRMHumanBoneName.Chest,
  "mixamorig:Spine2": VRMHumanBoneName.UpperChest,
  "mixamorig:Neck": VRMHumanBoneName.Neck,
  "mixamorig:Head": VRMHumanBoneName.Head,
  "mixamorig:LeftShoulder": VRMHumanBoneName.LeftShoulder,
  "mixamorig:LeftArm": VRMHumanBoneName.LeftUpperArm,
  "mixamorig:LeftForeArm": VRMHumanBoneName.LeftLowerArm,
  "mixamorig:LeftHand": VRMHumanBoneName.LeftHand,
  "mixamorig:RightShoulder": VRMHumanBoneName.RightShoulder,
  "mixamorig:RightArm": VRMHumanBoneName.RightUpperArm,
  "mixamorig:RightForeArm": VRMHumanBoneName.RightLowerArm,
  "mixamorig:RightHand": VRMHumanBoneName.RightHand,
  "mixamorig:LeftUpLeg": VRMHumanBoneName.LeftUpperLeg,
  "mixamorig:LeftLeg": VRMHumanBoneName.LeftLowerLeg,
  "mixamorig:LeftFoot": VRMHumanBoneName.LeftFoot,
  "mixamorig:LeftToeBase": VRMHumanBoneName.LeftToes,
  "mixamorig:RightUpLeg": VRMHumanBoneName.RightUpperLeg,
  "mixamorig:RightLeg": VRMHumanBoneName.RightLowerLeg,
  "mixamorig:RightFoot": VRMHumanBoneName.RightFoot,
  "mixamorig:RightToeBase": VRMHumanBoneName.RightToes,
};

const fbxLoader = new FBXLoader();
const clipCache = new Map<string, Promise<THREE.AnimationClip>>();

/** Load (or hit the cache for) a Mixamo .fbx and return an
 *  AnimationClip retargeted to the given VRM. */
export function loadGestureClip(url: string, vrm: VRM): Promise<THREE.AnimationClip> {
  const key = `${url}::${vrm.scene.uuid}`;
  const hit = clipCache.get(key);
  if (hit) return hit;
  const p = fbxLoader
    .loadAsync(url)
    .then((asset) => {
      if (!asset.animations || asset.animations.length === 0) {
        throw new Error(`no animation track in ${url}`);
      }
      return retargetMixamoClip(asset.animations[0], vrm);
    })
    .catch((e) => {
      // Drop bad entries so a transient network blip doesn't pin
      // the failure forever — next call retries.
      clipCache.delete(key);
      throw e;
    });
  clipCache.set(key, p);
  return p;
}

function retargetMixamoClip(clip: THREE.AnimationClip, vrm: VRM): THREE.AnimationClip {
  const tracks: THREE.KeyframeTrack[] = [];
  for (const track of clip.tracks) {
    const dotIdx = track.name.lastIndexOf(".");
    if (dotIdx <= 0) continue;
    const boneName = track.name.slice(0, dotIdx);
    const property = track.name.slice(dotIdx + 1);
    const vrmBoneName = MIXAMO_TO_VRM[boneName];
    if (!vrmBoneName) continue;
    const vrmNode = vrm.humanoid?.getNormalizedBoneNode(vrmBoneName);
    if (!vrmNode) continue;
    // Skip scale (Mixamo always emits 1,1,1) and non-hip position
    // (skeleton-length mismatch causes drift).
    if (property === "scale") continue;
    if (property === "position" && vrmBoneName !== VRMHumanBoneName.Hips) continue;
    const cloned = track.clone();
    cloned.name = `${vrmNode.name}.${property}`;
    tracks.push(cloned);
  }
  return new THREE.AnimationClip(clip.name, clip.duration, tracks);
}

export type GestureName =
  | "auto"
  | "acknowledging"
  | "angry"
  | "annoyed-shake"
  | "being-cocky"
  | "dismissing"
  | "happy-hand"
  | "hard-nod"
  | "lengthy-nod"
  | "look-away"
  | "nod-yes"
  | "relieved-sigh"
  | "sarcastic-nod"
  | "shake-no"
  | "thoughtful-shake"
  | "weight-shift";

/** Catalog shown in the settings dropdown. Order is roughly
 *  "most teacher-appropriate first." */
export const GESTURES: { name: GestureName; label: string; hint: string }[] = [
  { name: "auto",            label: "auto (emotion 連動)", hint: "[joy]/[relaxed]/[neutral]/[sad]/[anger] に合わせて自動切替" },
  { name: "acknowledging",   label: "acknowledging",        hint: "穏やかな相づち" },
  { name: "happy-hand",      label: "happy hand",           hint: "両手で説明、明るく" },
  { name: "nod-yes",         label: "nod yes",              hint: "頷き" },
  { name: "lengthy-nod",     label: "lengthy nod",          hint: "長めの頷き" },
  { name: "hard-nod",        label: "hard nod",             hint: "強い同意" },
  { name: "thoughtful-shake",label: "thoughtful shake",     hint: "考え込み" },
  { name: "shake-no",        label: "shake no",             hint: "否定の首振り" },
  { name: "relieved-sigh",   label: "relieved sigh",        hint: "ほっとした息" },
  { name: "look-away",       label: "look away",            hint: "視線そらし" },
  { name: "dismissing",      label: "dismissing",           hint: "軽くあしらう" },
  { name: "annoyed-shake",   label: "annoyed shake",        hint: "イライラ" },
  { name: "angry",           label: "angry",                hint: "怒り" },
  { name: "sarcastic-nod",   label: "sarcastic nod",        hint: "皮肉な頷き" },
  { name: "being-cocky",     label: "being cocky",          hint: "偉そう" },
  { name: "weight-shift",    label: "weight shift",         hint: "体重移動 (idle 系)" },
];

// Keys are the VRM expression names (after `extractEmotions`
// translates `[joy]` -> "happy" etc.), not the raw tag tokens.
const EMOTION_TO_GESTURE: Record<Emotion, GestureName> = {
  happy: "happy-hand",
  relaxed: "acknowledging",
  neutral: "nod-yes",
  sad: "thoughtful-shake",
  angry: "angry",
  surprised: "lengthy-nod",
};

/** Resolve "auto" against the current emotion; everything else
 *  passes through unchanged. */
export function effectiveGesture(picked: GestureName, emotion: Emotion): GestureName {
  return picked === "auto" ? EMOTION_TO_GESTURE[emotion] : picked;
}

export function gestureUrl(name: GestureName): string {
  return `/talk-gestures/${name}.fbx`;
}
