import { createContext, useContext } from "react";
import type { User } from "../../api/types";

export const ProfileContext = createContext<User | null>(null);

export function useProfile(): User {
  const profile = useContext(ProfileContext);
  if (!profile)
    throw new Error("Profile settings require an authenticated user");
  return profile;
}
