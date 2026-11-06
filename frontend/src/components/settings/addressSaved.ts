import type { User } from "../../types";

type Translate = (key: string, values?: Record<string, unknown>) => string;

/**
 * The toast after a new address was saved. With mail on, a link is on its
 * way. With mail off (`ServiceConfig.mail`) none comes: the address is saved
 * as it is when the account had none or an unconfirmed one, and otherwise
 * waits for a confirmation that this server cannot send yet.
 */
export function addressSavedMessage(t: Translate, mail: boolean, me: User | null, address: string): string {
  if (mail) return t("account.emailSent", { email: address });
  return me?.email === address ? t("account.emailSaved") : t("account.emailNotConfirmed");
}
