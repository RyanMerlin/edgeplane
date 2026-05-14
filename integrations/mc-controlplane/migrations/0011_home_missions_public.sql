-- Home missions were initially created as private, which hides them from the
-- mission list for human users. Flip them to public so the TUI Missions tab
-- can display them without needing an owner-subject match.
-- Only missions that are the home_mission of some agent and are currently
-- private are updated; user-created private missions are left alone.
UPDATE mission
   SET visibility = 'public'
 WHERE visibility = 'private'
   AND id IN (SELECT home_mission_id FROM agent WHERE home_mission_id IS NOT NULL);
