import { useEffect, useState } from "react";
import type { DetectedDir, DirValidation, HudProfile } from "../data/types";
import {
  completeOnboarding,
  detectPokerStarsDirs,
  getHudProfiles,
  pickFolderDialog,
  setActiveHudProfile,
  setHandHistoryDir,
  validateHandHistoryDir,
} from "../data/api";
import { POKERSTARS_TUTORIAL_NOTE, POKERSTARS_TUTORIAL_STEPS } from "./pokerStarsTutorial";
import styles from "./OnboardingFlow.module.css";

interface OnboardingFlowProps {
  onComplete: () => void;
}

type Step = 1 | 2 | 3 | 4;

const OTHER_ROOMS = ["GGPoker", "888poker", "partypoker", "iPoker"];

export function OnboardingFlow({ onComplete }: OnboardingFlowProps) {
  const [step, setStep] = useState<Step>(1);
  const [room, setRoom] = useState<"pokerstars" | null>(null);

  const [candidates, setCandidates] = useState<DetectedDir[] | null>(null);
  const [chosenDir, setChosenDir] = useState<string | null>(null);
  const [manualValidation, setManualValidation] = useState<DirValidation | null>(null);
  const [showTutorial, setShowTutorial] = useState(false);
  const [detecting, setDetecting] = useState(false);

  const [profiles, setProfiles] = useState<HudProfile[]>([]);
  const [selectedProfileId, setSelectedProfileId] = useState<string | null>(null);

  const [finishing, setFinishing] = useState(false);

  useEffect(() => {
    if (step === 2 && candidates === null) {
      setDetecting(true);
      detectPokerStarsDirs()
        .then((found) => {
          setCandidates(found);
          if (found.length > 0) {
            setChosenDir(found[0].path);
          }
        })
        .catch(() => setCandidates([]))
        .finally(() => setDetecting(false));
    }
    if (step === 3 && profiles.length === 0) {
      getHudProfiles()
        .then((all) => {
          setProfiles(all);
          setSelectedProfileId(all[0]?.id ?? null);
        })
        .catch(() => undefined);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

  async function handleChooseAnotherFolder() {
    const picked = await pickFolderDialog();
    if (!picked) return;
    const validation = await validateHandHistoryDir(picked);
    setManualValidation(validation);
    if (validation.isValid) {
      setChosenDir(picked);
    }
  }

  async function handleFinish() {
    setFinishing(true);
    try {
      if (chosenDir) {
        await setHandHistoryDir(chosenDir);
      }
      if (selectedProfileId) {
        await setActiveHudProfile(selectedProfileId);
      }
      await completeOnboarding(room ?? "pokerstars");
      setStep(4);
    } finally {
      setFinishing(false);
    }
  }

  const bestCandidate = candidates && candidates.length > 0 ? candidates[0] : null;

  return (
    <div className={styles.overlay}>
      <div className={styles.card}>
        <div className={styles.brand}>
          <div className={styles.mark}>V</div>
          <span>Welcome to Velora</span>
        </div>

        {step < 4 && <div className={styles.stepIndicator}>Step {step} / 3</div>}

        {step === 1 && (
          <div className={styles.stepBody}>
            <h1 className={styles.title}>Which poker room do you play on?</h1>
            <div className={styles.roomList}>
              <button
                type="button"
                className={`${styles.roomButton} ${room === "pokerstars" ? styles.roomButtonActive : ""}`}
                onClick={() => setRoom("pokerstars")}
              >
                PokerStars
              </button>
              {OTHER_ROOMS.map((r) => (
                <button key={r} type="button" className={styles.roomButtonDisabled} disabled>
                  {r} <span className={styles.comingSoon}>Coming soon</span>
                </button>
              ))}
            </div>
            <div className={styles.footer}>
              <button
                type="button"
                className={styles.primaryButton}
                disabled={room !== "pokerstars"}
                onClick={() => setStep(2)}
              >
                Continue
              </button>
            </div>
          </div>
        )}

        {step === 2 && (
          <div className={styles.stepBody}>
            <h1 className={styles.title}>Configure PokerStars</h1>

            {detecting && <p className={styles.detecting}>Scanning for your hand history folder&hellip;</p>}

            {!detecting && bestCandidate && (
              <div className={styles.detectedBox}>
                <div className={styles.detectedHeader}>
                  <span className={styles.checkmark}>&#10003;</span> Hand history detected
                </div>
                <div className={styles.detectedPath}>{bestCandidate.path}</div>
                <div className={styles.detectedMeta}>
                  {bestCandidate.handFileCount} hand file(s)
                  {bestCandidate.screenNames.length > 0 &&
                    ` · ${bestCandidate.screenNames.join(", ")}`}
                </div>
              </div>
            )}

            {!detecting && !bestCandidate && (
              <p className={styles.notFound}>
                We couldn&apos;t automatically find your PokerStars hand history. Choose the folder
                manually.
              </p>
            )}

            {chosenDir && chosenDir !== bestCandidate?.path && (
              <div className={styles.detectedBox}>
                <div className={styles.detectedHeader}>
                  <span className={styles.checkmark}>&#10003;</span> Folder selected
                </div>
                <div className={styles.detectedPath}>{chosenDir}</div>
                {manualValidation && (
                  <div className={styles.detectedMeta}>{manualValidation.message}</div>
                )}
              </div>
            )}

            <div className={styles.buttonRow}>
              {bestCandidate && (
                <button
                  type="button"
                  className={styles.primaryButton}
                  onClick={() => setChosenDir(bestCandidate.path)}
                >
                  Use detected folder
                </button>
              )}
              <button type="button" className={styles.secondaryButton} onClick={handleChooseAnotherFolder}>
                Choose another folder
              </button>
            </div>

            <button
              type="button"
              className={styles.tutorialToggle}
              onClick={() => setShowTutorial((v) => !v)}
            >
              How do I configure PokerStars?
            </button>

            {showTutorial && (
              <div className={styles.tutorialBox}>
                <ol className={styles.tutorialList}>
                  {POKERSTARS_TUTORIAL_STEPS.map((line, i) => (
                    <li key={i}>{line}</li>
                  ))}
                </ol>
                <p className={styles.tutorialNote}>{POKERSTARS_TUTORIAL_NOTE}</p>
              </div>
            )}

            <div className={styles.footer}>
              <button type="button" className={styles.linkButton} onClick={() => setStep(1)}>
                Back
              </button>
              <button
                type="button"
                className={styles.primaryButton}
                disabled={!chosenDir}
                onClick={() => setStep(3)}
              >
                Continue
              </button>
            </div>
          </div>
        )}

        {step === 3 && (
          <div className={styles.stepBody}>
            <h1 className={styles.title}>Choose your HUD</h1>
            <div className={styles.hudList}>
              {profiles.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  className={`${styles.hudButton} ${
                    selectedProfileId === p.id ? styles.hudButtonActive : ""
                  }`}
                  onClick={() => setSelectedProfileId(p.id)}
                >
                  {p.name}
                </button>
              ))}
            </div>
            <div className={styles.footer}>
              <button type="button" className={styles.linkButton} onClick={() => setStep(2)}>
                Back
              </button>
              <button
                type="button"
                className={styles.primaryButton}
                disabled={finishing || !selectedProfileId}
                onClick={handleFinish}
              >
                {finishing ? "Finishing…" : "Finish"}
              </button>
            </div>
          </div>
        )}

        {step === 4 && (
          <div className={styles.stepBody}>
            <h1 className={styles.title}>You&apos;re ready.</h1>
            <p className={styles.readyText}>Velora is watching your hand histories.</p>
            <div className={styles.footer}>
              <button type="button" className={styles.primaryButton} onClick={onComplete}>
                Open Velora
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
