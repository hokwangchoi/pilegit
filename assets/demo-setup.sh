#!/bin/bash
# Creates a demo git repo for recording pilegit GIF
set -e

DEMO_DIR="/tmp/pgit-demo"
rm -rf "$DEMO_DIR"
mkdir -p "$DEMO_DIR"
cd "$DEMO_DIR"

git init
git checkout -b main

git config user.name "hokwang"
git config user.email "demo@pilegit.dev"

# Base commit
cat > app.rs << 'EOF'
fn main() {
    println!("Hello, world!");
}
EOF
git add -A && git commit -m "initial setup"

# Self-referencing remote so pgit can detect base
git remote add origin "$DEMO_DIR"
git fetch origin 2>/dev/null || true

# --- Stack of 5 commits ---

cat > schema.sql << 'EOF'
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email VARCHAR(255) NOT NULL UNIQUE,
    created_at TIMESTAMP DEFAULT NOW()
);

CREATE TABLE sessions (
    id SERIAL PRIMARY KEY,
    user_id INTEGER REFERENCES users(id),
    token VARCHAR(512) NOT NULL,
    expires_at TIMESTAMP NOT NULL
);
EOF
git add -A && git commit -m "feat: add database schema for users and sessions"

cat > auth.rs << 'EOF'
pub fn verify_token(token: &str) -> Result<UserId, AuthError> {
    let claims = decode_jwt(token)?;
    if claims.exp < Utc::now().timestamp() {
        return Err(AuthError::Expired);
    }
    Ok(claims.sub)
}

pub fn create_session(user_id: UserId) -> Session {
    Session {
        token: generate_token(user_id),
        expires_at: Utc::now() + Duration::hours(24),
    }
}
EOF
git add -A && git commit -m "feat: implement JWT auth middleware"

cat > api.rs << 'EOF'
#[get("/api/profile")]
async fn get_profile(auth: Auth) -> Json<Profile> {
    let user = db::find_user(auth.user_id).await?;
    Json(Profile {
        email: user.email,
        name: user.name,
        avatar_url: user.avatar_url,
    })
}

#[put("/api/profile")]
async fn update_profile(auth: Auth, body: Json<UpdateProfile>) -> Json<Profile> {
    let user = db::update_user(auth.user_id, body.into_inner()).await?;
    Json(user.into())
}
EOF
git add -A && git commit -m "feat: add user profile API endpoints"

cat > rate_limit.rs << 'EOF'
pub struct RateLimiter {
    window: Duration,
    max_requests: u32,
    store: DashMap<IpAddr, Vec<Instant>>,
}

impl RateLimiter {
    pub fn check(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        let mut timestamps = self.store.entry(ip).or_default();
        let cutoff = Instant::now() - self.window;
        timestamps.retain(|t| *t > cutoff);
        if timestamps.len() >= self.max_requests as usize {
            return Err(RateLimitError::TooManyRequests);
        }
        timestamps.push(Instant::now());
        Ok(())
    }
}
EOF
git add -A && git commit -m "feat: add rate limiting middleware"

cat > dashboard.rs << 'EOF'
#[get("/api/dashboard")]
async fn dashboard(auth: Auth) -> Json<DashboardData> {
    let stats = db::get_user_stats(auth.user_id).await?;
    let recent = db::get_recent_activity(auth.user_id, 10).await?;
    Json(DashboardData {
        total_logins: stats.login_count,
        last_seen: stats.last_login,
        recent_activity: recent,
    })
}
EOF
git add -A && git commit -m "feat: dashboard analytics endpoint"

cat > .pilegit.toml << 'EOF'
[forge]
type = "github"

[repo]
base = "origin/main"
EOF

echo ""
echo "✓ Demo repo ready at: $DEMO_DIR"
echo "  cd $DEMO_DIR && pgit"
