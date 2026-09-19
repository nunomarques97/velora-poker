import { useEffect, useState } from "react";
import type { DetectedDir, DirValidation, HudProfile, OnboardingReadinessPayload } from "../data/types";
import {
  completeOnboarding,
  detectPokerStarsDirs,
  getHudProfiles,
  getOnboardingReadiness,
  pickFolderDialog,
  setActiveHudProfile,
  setAutoCenterEnabled,
  setHandHistoryDir,
  validateHandHistoryDir,
} from "../data/api";
import { POKERSTARS_TUTORIAL_NOTE, POKERSTARS_TUTORIAL_STEPS } from "./pokerStarsTutorial";
import styles from "./OnboardingFlow.module.css";

interface OnboardingFlowProps {
  onComplete: () => void;
}

type Step = 1 | 2 | 3 | 4 | 5;

type ReadinessStatus = "checking" | "green" | "red" | "skipped";

const OTHER_ROOMS = ["GGPoker", "888poker", "partypoker", "iPoker"];

// The exact cost of each skip, written once so the copy can never drift
// from the button that triggers it — a skip button never renders without its
// cost right next to it.
const SKIP_COSTS = {
  folder: "Velora won't see a single hand.",
  language: "Hands may be misread silently.",
  autoCenter: "HUD cards may appear in the wrong seats.",
} as const;

// The exact PokerStars step that turns on Auto-Center, pulled from the same
// data pokerStarsTutorial.ts already exports for step 2 — never re-typed.
//
// Contract with pokerStarsTutorial.ts: the Auto-Center step is identified by
// the literal "auto-center" in its own text. If that wording is ever changed
// there, this degrades LOUDLY — the whole tutorial list is shown instead —
// never silently into an item with no instructions at all.
const AUTO_CENTER_MATCH = POKERSTARS_TUTORIAL_STEPS.find((line) =>
  line.toLowerCase().includes("auto-center"),
);
const AUTO_CENTER_STEPS: string[] = AUTO_CENTER_MATCH
  ? [AUTO_CENTER_MATCH]
  : POKERSTARS_TUTORIAL_STEPS;

function describeError(err: unknown): string {
  if (typeof err === "string" && err.trim().length > 0) return err;
  if (err instanceof Error && err.message.trim().length > 0) return err.message;
  return "the check could not be run.";
}

function statusIcon(status: ReadinessStatus): string {
  switch (status) {
    case "green":
      return "✓"; // checkmark, same glyph as the step-2 detected box
    case "red":
      return "✕"; // ✕
    case "skipped":
      return "→"; // →
    default:
      return "⋯"; // ⋯
  }
}

function statusLabel(status: ReadinessStatus): string {
  switch (status) {
    case "green":
      return "Ready";
    case "red":
      return "Needs attention";
    case "skipped":
      return "Skipped";
    default:
      return "Checking…";
  }
}

function statusClassName(status: ReadinessStatus): string {
  switch (status) {
    case "green":
      return styles.readinessItemGreen;
    case "red":
      return styles.readinessItemRed;
    case "skipped":
      return styles.readinessItemSkipped;
    default:
      return styles.readinessItemChecking;
  }
}

interface ReadinessItemProps {
  title: string;
  status: ReadinessStatus;
  children: React.ReactNode;
  skipCost: string;
  onSkip: () => void;
  onUndoSkip: () => void;
  skipped: boolean;
}

function ReadinessItem({
  title,
  status,
  children,
  skipCost,
  onSkip,
  onUndoSkip,
  skipped,
}: ReadinessItemProps) {
  return (
    <div className={`${styles.readinessItem} ${statusClassName(status)}`}>
      <div className={styles.readinessItemHeader}>
        <span className={styles.readinessItemIcon} aria-hidden="true">
          {statusIcon(status)}
        </span>
        <span className={styles.readinessItemTitle}>{title}</span>
        <span className={styles.readinessItemStatusLabel}>{statusLabel(status)}</span>
      </div>
      <div className={styles.readinessItemBody}>{children}</div>
      {status === "red" && !skipped && (
        <div className={styles.skipRow}>
          <span className={styles.skipCost}>{skipCost}</span>
          <button type="button" className={styles.skipButton} onClick={onSkip}>
            Skip
          </button>
        </div>
      )}
      {skipped && status !== "green" && (
        <div className={styles.skipRow}>
          <span className={styles.skipCost}>Skipped &mdash; {skipCost}</span>
          <button type="button" className={styles.undoSkipButton} onClick={onUndoSkip}>
            Undo skip
          </button>
        </div>
      )}
    </div>
  );
}

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

  const [savingConfig, setSavingConfig] = useState(false);
  const [finishing, setFinishing] = useState(false);
  const [finishError, setFinishError] = useState<string | null>(null);

  // --- Readiness gate (step 4) ---
  const [readiness, setReadiness] = useState<OnboardingReadinessPayload | null>(null);
  const [readinessLoading, setReadinessLoading] = useState(false);
  const [readinessError, setReadinessError] = useState<string | null>(null);
  const [skipped, setSkipped] = useState({ folder: false, language: false, autoCenter: false });
  const [confirmingAutoCenter, setConfirmingAutoCenter] = useState(false);

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

  async function fetchReadiness() {
    setReadinessLoading(true);
    setReadinessError(null);
    try {
      const result = await getOnboardingReadiness(chosenDir);
      setReadiness(result);
    } catch (err) {
      setReadiness(null);
      // Tauri rejects a command with its own error payload, usually a plain
      // string — swallowing it into a generic sentence would hide the one
      // piece of information the user could act on.
      setReadinessError(describeError(err));
    } finally {
      setReadinessLoading(false);
    }
  }

  useEffect(() => {
    if (step === 4 && readiness === null && !readinessLoading) {
      fetchReadiness();
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

  async function handleContinueToReadiness() {
    setSavingConfig(true);
    try {
      if (chosenDir) {
        await setHandHistoryDir(chosenDir);
      }
      if (selectedProfileId) {
        await setActiveHudProfile(selectedProfileId);
      }
      setStep(4);
    } finally {
      setSavingConfig(false);
    }
  }

  async function handleConfirmAutoCenter() {
    setConfirmingAutoCenter(true);
    try {
      await setAutoCenterEnabled(true);
      await fetchReadiness();
    } finally {
      setConfirmingAutoCenter(false);
    }
  }

  async function handleOpenVelora() {
    setFinishing(true);
    setFinishError(null);
    try {
      await completeOnboarding(room ?? "pokerstars");
      onComplete();
    } catch (err) {
      // A click that does nothing and says nothing is the same defect as a
      // dead button: the failure has to be on screen.
      setFinishError(describeError(err));
    } finally {
      setFinishing(false);
    }
  }

  const bestCandidate = candidates && candidates.length > 0 ? candidates[0] : null;

  // --- Readiness derivation ---
  const folderOk = readiness ? readiness.folder.exists && readiness.folder.parsedHandCount > 0 : false;
  const languageOk = readiness ? readiness.clientLanguage.checked && readiness.clientLanguage.isEnglish : false;
  const autoCenterOk = readiness ? readiness.autoCenter.enabled : false;

  function itemStatus(ok: boolean, itemSkipped: boolean): ReadinessStatus {
    if (readinessLoading && !readiness) return "checking";
    if (ok) return "green";
    if (itemSkipped) return "skipped";
    return "red";
  }

  const folderStatus = itemStatus(folderOk, skipped.folder);
  const languageStatus = itemStatus(languageOk, skipped.language);
  const autoCenterStatus = itemStatus(autoCenterOk, skipped.autoCenter);

  const stillChecking =
    folderStatus === "checking" || languageStatus === "checking" || autoCenterStatus === "checking";

  const blockingLabels: string[] = [];
  if (folderStatus === "red") blockingLabels.push("hand history folder");
  if (languageStatus === "red") blockingLabels.push("PokerStars language");
  if (autoCenterStatus === "red") blockingLabels.push("Auto-Center");

  // "Green, or explicitly skipped" is the whole gate.
  // Deliberately NOT conditioned on `readiness !== null`: a failed
  // `get_onboarding_readiness` leaves the payload null, the three items red,
  // and — with that extra condition — the user who skipped all three stuck
  // behind a permanently dead "Open Velora" with no way out.
  // `folderOk`/`languageOk`/`autoCenterOk` are already false
  // without a payload, so nothing can turn green without real backend data.
  const allReady = !stillChecking && blockingLabels.length === 0;

  // The three greens, ignoring skips — the only state that lets the final
  // screen claim Velora is actually working.
  const allGreen =
    folderStatus === "green" && languageStatus === "green" && autoCenterStatus === "green";

  const skippedItems: { label: string; cost: string }[] = [];
  if (folderStatus === "skipped")
    skippedItems.push({ label: "Hand history folder", cost: SKIP_COSTS.folder });
  if (languageStatus === "skipped")
    skippedItems.push({ label: "PokerStars client in English", cost: SKIP_COSTS.language });
  if (autoCenterStatus === "skipped")
    skippedItems.push({ label: "Auto-Center", cost: SKIP_COSTS.autoCenter });

  // Never a disabled button without a written reason (product profile,
  // "Inaceitável" #3). Every branch that can disable "Open Velora" has copy.
  const openBlockedReason = allReady
    ? null
    : stillChecking
      ? "Still checking your setup — one moment."
      : blockingLabels.length > 0
        ? `Go back and confirm or skip: ${blockingLabels.join(", ")}.`
        : "Velora couldn't finish the checks. Go back and press “Check again”.";

  // With no payload the three items are red, not blank: each body says why the
  // check could not run, so the Skip button next to it is an informed choice.
  const readinessUnavailableNote = readinessError
    ? `Velora couldn't run this check: ${readinessError}`
    : "Checking…";

  return (
    <div className={styles.overlay}>
      {/* On steps 2 and 4 the card becomes a column with a pinned
          action row (see OnboardingFlow.module.css). Every other step keeps the
          card's own scroll, untouched. */}
      <div
        className={`${styles.card} ${step === 2 || step === 4 ? styles.cardPinnedFooter : ""}`}
      >
        <div className={styles.brand}>
          <div className={styles.mark}>V</div>
          <span>Welcome to Velora</span>
        </div>

        {step < 5 && <div className={styles.stepIndicator}>Step {step} / 4</div>}

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
            <div className={styles.stepScroll}>
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
            </div>

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
                disabled={savingConfig || !selectedProfileId}
                onClick={handleContinueToReadiness}
              >
                {savingConfig ? "Saving…" : "Continue"}
              </button>
            </div>
          </div>
        )}

        {step === 4 && (
          <div className={styles.stepBody}>
            <div className={styles.stepScroll}>
              <h1 className={styles.title}>Let&apos;s make sure it works</h1>
              <p className={styles.readinessIntro}>
                Velora checks three things before it can trust what it shows you. Anything you skip
                here has a real cost &mdash; it&apos;s written next to the button.
              </p>

              {readinessError && (
                <p className={styles.readinessErrorText}>
                  Failed to check your setup: {readinessError}
                </p>
              )}

              <div className={styles.readinessList}>
                <ReadinessItem
                  title="Hand history folder"
                  status={folderStatus}
                  skipCost={SKIP_COSTS.folder}
                  skipped={skipped.folder}
                  onSkip={() => setSkipped((s) => ({ ...s, folder: true }))}
                  onUndoSkip={() => setSkipped((s) => ({ ...s, folder: false }))}
                >
                  {readiness ? (
                    <>
                      <div className={styles.readinessDetailPath}>
                        {readiness.folder.path ?? "No folder found"}
                      </div>
                      <div className={styles.readinessDetailStat}>
                        Hands read:{" "}
                        <span className={`${styles.readinessDetailStatValue} tabular`}>
                          {readiness.folder.parsedHandCount}
                        </span>
                      </div>
                      <div className={styles.readinessDetailNote}>{readiness.folder.message}</div>
                    </>
                  ) : (
                    <div className={styles.readinessDetailNote}>{readinessUnavailableNote}</div>
                  )}
                </ReadinessItem>

                <ReadinessItem
                  title="PokerStars client in English"
                  status={languageStatus}
                  skipCost={SKIP_COSTS.language}
                  skipped={skipped.language}
                  onSkip={() => setSkipped((s) => ({ ...s, language: true }))}
                  onUndoSkip={() => setSkipped((s) => ({ ...s, language: false }))}
                >
                  {readiness ? (
                    <>
                      <div className={styles.readinessDetailNote}>{readiness.clientLanguage.reason}</div>
                      {readiness.clientLanguage.sampleFile && (
                        <div className={styles.readinessDetailPath}>
                          Checked against: {readiness.clientLanguage.sampleFile}
                        </div>
                      )}
                    </>
                  ) : (
                    <div className={styles.readinessDetailNote}>{readinessUnavailableNote}</div>
                  )}
                </ReadinessItem>

                <ReadinessItem
                  title="Auto-Center"
                  status={autoCenterStatus}
                  skipCost={SKIP_COSTS.autoCenter}
                  skipped={skipped.autoCenter}
                  onSkip={() => setSkipped((s) => ({ ...s, autoCenter: true }))}
                  onUndoSkip={() => setSkipped((s) => ({ ...s, autoCenter: false }))}
                >
                  {autoCenterOk ? (
                    <div className={styles.readinessDetailNote}>Auto-Center is on.</div>
                  ) : autoCenterStatus === "checking" ? (
                    <div className={styles.readinessDetailNote}>Checking&hellip;</div>
                  ) : (
                    <>
                      {AUTO_CENTER_STEPS.map((line) => (
                        <div key={line} className={styles.readinessDetailNote}>
                          {line}
                        </div>
                      ))}
                      <button
                        type="button"
                        className={`${styles.secondaryButton} ${styles.autoCenterConfirmButton}`}
                        onClick={handleConfirmAutoCenter}
                        disabled={confirmingAutoCenter}
                      >
                        {confirmingAutoCenter ? "Checking…" : "I've enabled it in PokerStars"}
                      </button>
                    </>
                  )}
                </ReadinessItem>
              </div>
            </div>

            <div className={styles.footer}>
              <button type="button" className={styles.linkButton} onClick={() => setStep(3)}>
                Back
              </button>
              <div className={styles.readinessActions}>
                <button
                  type="button"
                  className={styles.secondaryButton}
                  onClick={fetchReadiness}
                  disabled={readinessLoading}
                >
                  {readinessLoading ? "Checking…" : "Check again"}
                </button>
                <button type="button" className={styles.primaryButton} onClick={() => setStep(5)}>
                  Continue
                </button>
              </div>
            </div>
          </div>
        )}

        {step === 5 && (
          <div className={styles.stepBody}>
            <h1 className={styles.title}>
              {allGreen
                ? "You're ready."
                : allReady
                  ? `Ready, with ${skippedItems.length} check${skippedItems.length === 1 ? "" : "s"} skipped.`
                  : "Almost ready."}
            </h1>
            <p className={styles.readyText}>
              {allGreen
                ? "Velora is watching your hand histories."
                : allReady
                  ? "Velora will open, but it can't promise these will work:"
                  : "Some checks still need your attention before Velora can be trusted."}
            </p>
            {allReady && !allGreen && (
              <ul className={styles.skippedSummary}>
                {skippedItems.map((item) => (
                  <li key={item.label}>
                    <span className={styles.skippedSummaryLabel}>{item.label}</span> &mdash;{" "}
                    <span className={styles.skippedSummaryCost}>{item.cost}</span>
                  </li>
                ))}
              </ul>
            )}
            {finishError && (
              <p className={styles.readinessErrorText}>
                Failed to finish setup: {finishError}
              </p>
            )}
            <div className={styles.footer}>
              <button type="button" className={styles.linkButton} onClick={() => setStep(4)}>
                Back
              </button>
              <div className={styles.readinessActions}>
                {openBlockedReason && (
                  <span className={styles.disabledReason}>{openBlockedReason}</span>
                )}
                <button
                  type="button"
                  className={styles.primaryButton}
                  disabled={!allReady || finishing}
                  onClick={handleOpenVelora}
                >
                  {finishing ? "Opening…" : "Open Velora"}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
