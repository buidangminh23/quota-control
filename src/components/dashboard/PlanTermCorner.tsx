/**
 * The card header's right corner, beside the plan and the email: how long the plan's paid period has
 * left and the day it ends, in the warning color from three days out. The hover note gives the exact
 * time and whether the service confirmed the date or it is an estimate.
 */
import type { PlanTermLines } from "@/model/planTermLines";
import { tooltipProps } from "../ui/tooltip";

export function PlanTermCorner({ lines }: { lines: PlanTermLines }) {
  return (
    <div className={`uc-plan-term${lines.soon ? " is-soon" : ""}`} role="group" aria-label={`${lines.left}. ${lines.day}. ${lines.note}`} {...tooltipProps(lines.note)}>
      <span className="uc-plan-term-left uc-num">
        <span>{lines.left}</span>
      </span>
      <span className="uc-plan-term-day uc-num">{lines.day}</span>
    </div>
  );
}
