import { useEffect, useRef } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { VRMLoaderPlugin, type VRM, VRMUtils, VRMHumanBoneName } from "@pixiv/three-vrm";
import {
  VRMAnimationLoaderPlugin,
  type VRMAnimation,
  createVRMAnimationClip,
} from "@pixiv/three-vrm-animation";

type Props = {
  /** Path to the VRM file (served by the backend under /avatars/...). */
  src: string;
  /** Optional .vrma idle animation. Falls back to a manual rest pose if missing/404. */
  animationSrc?: string;
  /** 0-1 mouth open value, driven by TTS in the parent component. */
  mouth: number;
  /** Emotion preset key; see VRM Expression names. */
  emotion?: "neutral" | "happy" | "sad" | "surprised" | "angry" | "relaxed";
};

export default function VrmViewer({
  src,
  animationSrc = "/avatars/idle.vrma",
  mouth,
  emotion = "neutral",
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const vrmRef = useRef<VRM | null>(null);
  const mouthRef = useRef(0);
  const emotionRef = useRef(emotion);

  useEffect(() => { mouthRef.current = mouth; }, [mouth]);
  useEffect(() => { emotionRef.current = emotion; }, [emotion]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const scene = new THREE.Scene();
    scene.background = null;

    const camera = new THREE.PerspectiveCamera(28, 1, 0.1, 20);
    camera.position.set(0, 1.35, 1.8);
    camera.lookAt(0, 1.35, 0);

    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    host.appendChild(renderer.domElement);

    const key = new THREE.DirectionalLight(0xffffff, 1.4);
    key.position.set(1.5, 2.5, 1.5);
    scene.add(key);
    scene.add(new THREE.AmbientLight(0xffffff, 0.7));

    const resize = () => {
      const w = host.clientWidth;
      const h = host.clientHeight;
      // updateStyle=true (default) so the canvas's CSS size matches the
      // container.  Passing `false` made the canvas render at the raw
      // devicePixelRatio×size and overflow on mobile (half-cropped avatar).
      renderer.setSize(w, h);
      camera.aspect = w / Math.max(1, h);
      camera.updateProjectionMatrix();
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(host);

    const loader = new GLTFLoader();
    loader.register((parser) => new VRMLoaderPlugin(parser));
    loader.register((parser) => new VRMAnimationLoaderPlugin(parser));

    let cancelled = false;
    let mixer: THREE.AnimationMixer | null = null;
    loader.load(
      src,
      async (gltf) => {
        if (cancelled) return;
        const vrm = gltf.userData.vrm as VRM | undefined;
        if (!vrm) return;
        // VRM 0.x models face -Z; rotate so the avatar faces the camera.
        VRMUtils.rotateVRM0(vrm);
        scene.add(vrm.scene);
        vrmRef.current = vrm;

        // Try to load a .vrma idle animation. Falls back to manual rest pose.
        try {
          const animGltf = await loader.loadAsync(animationSrc);
          const anims = animGltf.userData.vrmAnimations as VRMAnimation[] | undefined;
          if (anims && anims.length > 0) {
            const clip = createVRMAnimationClip(anims[0], vrm);
            mixer = new THREE.AnimationMixer(vrm.scene);
            mixer.clipAction(clip).play();
            return;
          }
        } catch {
          // no idle.vrma — fall through to manual pose
        }
        applyRestPose(vrm);
      },
      undefined,
      (err) => console.error("VRM load failed", err),
    );

    const clock = new THREE.Clock();
    let raf = 0;
    let nextBlinkAt = 2 + Math.random() * 3;
    let blinkPhase = -1; // -1 = idle, 0..1 = blinking progress
    const tick = () => {
      const dt = clock.getDelta();
      if (mixer) mixer.update(dt);
      const vrm = vrmRef.current;
      if (vrm) {
        const t = clock.elapsedTime;
        const exp = vrm.expressionManager;
        if (exp) {
          // Mouth shape — VRM 1.0 uses "aa", 0.x uses "a"; setting both is harmless.
          exp.setValue("aa", mouthRef.current);
          exp.setValue("a",  mouthRef.current);
          // Drive every VRM standard emotion expression. "neutral"
          // intentionally has no slot — it's the absence of the
          // others, which collapses to a blank face.
          for (const e of ["happy", "sad", "surprised", "angry", "relaxed"]) {
            exp.setValue(e, emotionRef.current === e ? 1 : 0);
          }
          // Blink: ~once every 3-5s, ~150ms close+open
          if (blinkPhase < 0 && t > nextBlinkAt) {
            blinkPhase = 0;
          }
          if (blinkPhase >= 0) {
            blinkPhase += dt / 0.15;
            const v = blinkPhase < 0.5
              ? blinkPhase * 2          // closing
              : (1 - blinkPhase) * 2;   // opening
            exp.setValue("blink", Math.max(0, Math.min(1, v)));
            if (blinkPhase >= 1) {
              blinkPhase = -1;
              exp.setValue("blink", 0);
              nextBlinkAt = t + 3 + Math.random() * 3;
            }
          }
        }
        // Tiny idle motion — applied on top of VRMA so the avatar
        // still looks alive when she's "still".
        if (!mixer) {
          vrm.scene.position.y = Math.sin(t * 1.3) * 0.005;
          const head = vrm.humanoid?.getNormalizedBoneNode(VRMHumanBoneName.Head);
          if (head) {
            head.rotation.y = Math.sin(t * 0.4) * 0.06;
            head.rotation.x = Math.sin(t * 0.7) * 0.03;
          }
        }
        vrm.update(dt);
      }
      renderer.render(scene, camera);
      raf = requestAnimationFrame(tick);
    };
    tick();

    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      ro.disconnect();
      renderer.dispose();
      host.removeChild(renderer.domElement);
      if (vrmRef.current) {
        VRMUtils.deepDispose(vrmRef.current.scene);
        vrmRef.current = null;
      }
    };
  }, [src]);

  return <div ref={hostRef} className="h-full w-full" />;
}

/** Drop the arms from the bind pose (T-pose) to a natural standing rest.
 *  VRM normalized bones have a known canonical orientation, so the same
 *  rotation values look right across all VRM models — no per-model tuning.
 */
function applyRestPose(vrm: VRM) {
  const h = vrm.humanoid;
  if (!h) return;
  const deg = THREE.MathUtils.degToRad;
  const set = (name: VRMHumanBoneName, x = 0, y = 0, z = 0) => {
    const node = h.getNormalizedBoneNode(name);
    if (node) node.rotation.set(deg(x), deg(y), deg(z));
  };
  // Arms down by the sides — VRM normalized bones rotate the OPPOSITE way
  // from naive intuition (positive Z on the left arm goes UP/banzai, not down).
  set(VRMHumanBoneName.LeftUpperArm,  0,  0, -70);
  set(VRMHumanBoneName.RightUpperArm, 0,  0,  70);
  set(VRMHumanBoneName.LeftLowerArm,  0,  8, -10);
  set(VRMHumanBoneName.RightLowerArm, 0, -8,  10);
  set(VRMHumanBoneName.LeftHand,      0, 0,   0);
  set(VRMHumanBoneName.RightHand,     0, 0,   0);
}
