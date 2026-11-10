-- Seed data: default roles, permissions and their mapping.

-- -----------------------------------------------------------------------------
-- 13. SEED DATA  (sensible defaults; edit per project)
-- -----------------------------------------------------------------------------
INSERT INTO roles (name, description, is_system) VALUES
    ('super_admin', 'Full unrestricted access',          1),
    ('admin',       'Manage users and settings',         1),
    ('moderator',   'Moderate content and users',        1),
    ('user',        'Default authenticated user',        1);

INSERT INTO permissions (name, description) VALUES
    ('*',            'Wildcard: everything'),
    ('users:read',   'View users'),
    ('users:write',  'Create / update users'),
    ('users:delete', 'Delete users'),
    ('roles:read',   'View roles'),
    ('roles:write',  'Manage roles and assignments'),
    ('audit:read',   'View audit log'),
    ('profile:read', 'Read own profile'),
    ('profile:write','Update own profile');

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r, permissions p
 WHERE (r.name = 'super_admin' AND p.name = '*')
    OR (r.name = 'admin'       AND p.name IN ('users:read','users:write','roles:read','audit:read','profile:read','profile:write'))
    OR (r.name = 'moderator'   AND p.name IN ('users:read','profile:read','profile:write'))
    OR (r.name = 'user'        AND p.name IN ('profile:read','profile:write'));
