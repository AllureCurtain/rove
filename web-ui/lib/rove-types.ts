export type RunStatus = "init" | "running" | "done" | "error" | "cancelled" | "interrupted";

export interface Usage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

export interface PlanStep {
  id: string;
  title: string;
  done: boolean;
}

export interface TaskPlan {
  goal: string;
  steps: PlanStep[];
  current_step: number;
}

export interface ToolResult {
  call_id: string;
  output: string;
}

export interface ToolError {
  code: string;
  reason?: string;
  timeout_ms?: number;
  name?: string;
  [key: string]: unknown;
}

export type StreamEvent =
  | {
      type: "run_started";
      run_id: string;
      job_id: string;
      user_message: string;
    }
  | {
      type: "llm_chunk";
      delta: string;
    }
  | {
      type: "llm_message";
      full: string;
      usage: Usage;
    }
  | {
      type: "tool_call_started";
      call_id: string;
      name: string;
      args: unknown;
    }
  | {
      type: "tool_call_approval_needed";
      call_id: string;
      name: string;
      args: unknown;
      reason: string;
    }
  | {
      type: "tool_call_completed";
      call_id: string;
      result: ToolResult;
    }
  | {
      type: "tool_call_failed";
      call_id: string;
      error: ToolError;
    }
  | {
      type: "input_needed";
      input_id: string;
      prompt: string;
    }
  | {
      type: "plan_created";
      plan: TaskPlan;
    }
  | {
      type: "plan_step_started";
      step: PlanStep;
      index: number;
    }
  | {
      type: "plan_step_completed";
      step: PlanStep;
      index: number;
    }
  | {
      type: "plan_step_failed";
      step: PlanStep;
      index: number;
      reason: string;
    }
  | {
      type: "run_completed";
      reason: string;
      output?: string | null;
    };

export interface CreateJobRequest {
  message: string;
  model?: string;
  max_steps?: number;
  approval?: ApprovalPolicy;
}

export interface CreateJobResponse {
  job_id: string;
  run_id: string;
}

export interface JobStateResponse {
  job_id: string;
  run_id: string;
  status: RunStatus;
  event_count: number;
  events: JobStreamEvent[];
  pending_approvals: PendingApproval[];
  pending_inputs: PendingInput[];
}

export interface JobStreamEvent {
  seq: number;
  event: StreamEvent;
}

export interface PendingApproval {
  call_id: string;
  name: string;
  args: unknown;
  reason: string;
}

export interface PendingInput {
  input_id: string;
  prompt: string;
}

export type ApprovalDecision = "approve" | "reject";

export type ApprovalPolicy = "ask" | "auto" | "never";

export const STREAM_EVENT_NAMES = [
  "run_started",
  "llm_chunk",
  "llm_message",
  "tool_call_started",
  "tool_call_approval_needed",
  "tool_call_completed",
  "tool_call_failed",
  "input_needed",
  "plan_created",
  "plan_step_started",
  "plan_step_completed",
  "plan_step_failed",
  "run_completed",
] as const;

export type StreamEventName = (typeof STREAM_EVENT_NAMES)[number];
