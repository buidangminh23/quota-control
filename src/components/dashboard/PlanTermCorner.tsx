/**
 * The card header's right corner, beside the plan and the email: how long the plan's paid period has
 * left and the day it ends, in the warning color from three days out. Claude's day is an estimate
 * from the subscription start and says so; the hover note gives the exact time and the source.
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
