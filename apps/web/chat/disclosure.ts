/**
 * Open/closed state for a collapsible block.
 *
 * Automatic changes apply only until the reader acts. Once they expand or
 * collapse a block themselves, later automatic updates are ignored, so a card
 * the reader closed stays closed when the next tool event arrives.
 */

export interface DisclosureState {
  open: boolean;
  userControlled: boolean;
}

export type DisclosureEvent =
  | { type: "auto"; wantOpen: boolean }
  | { type: "user"; open: boolean };

export function initialDisclosure(wantOpen: boolean): DisclosureState {
  return { open: wantOpen, userControlled: false };
}

export function reduceDisclosure(state: DisclosureState, event: DisclosureEvent): DisclosureState {
  if (event.type === "user") {
    return { open: event.open, userControlled: true };
  }
  if (state.userControlled) {
    return state;
  }
  return { open: event.wantOpen, userControlled: false };
}
