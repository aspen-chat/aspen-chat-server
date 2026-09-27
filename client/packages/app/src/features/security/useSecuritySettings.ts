import { useCallback, useEffect, useState } from "react";
import { useAspenClient } from "@/api/context";
import * as api from "./api";

/** The caller's security settings, loaded on mount and again after every change. */
export function useSecuritySettings() {
  const client = useAspenClient();
  const [settings, setSettings] = useState<api.SecuritySettings | null>(null);
  const [failed, setFailed] = useState(false);
  const reload = useCallback(async () => {
    try {
      setSettings(await api.fetchSecurity(client));
      setFailed(false);
    } catch {
      setFailed(true);
    }
  }, [client]);
  useEffect(() => {
    let live = true;
    api
      .fetchSecurity(client)
      .then((loaded) => {
        if (live) {
          setSettings(loaded);
        }
      })
      .catch(() => {
        if (live) {
          setFailed(true);
        }
      });
    return () => {
      live = false;
    };
  }, [client]);
  return { settings, failed, reload };
}
