create table if not exists public.profiles (
  id uuid primary key references auth.users(id) on delete cascade,
  email text not null unique,
  password_set_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

alter table public.profiles enable row level security;

drop policy if exists "Users can read own profile" on public.profiles;
create policy "Users can read own profile"
on public.profiles
for select
to authenticated
using ((select auth.uid()) = id);

drop policy if exists "Users can insert own profile" on public.profiles;
create policy "Users can insert own profile"
on public.profiles
for insert
to authenticated
with check ((select auth.uid()) = id);

drop policy if exists "Users can update own profile" on public.profiles;
create policy "Users can update own profile"
on public.profiles
for update
to authenticated
using ((select auth.uid()) = id)
with check ((select auth.uid()) = id);

create or replace function public.get_auth_account_status(account_email text)
returns table(account_exists boolean, password_set boolean)
language sql
security definer
set search_path = ''
as $$
  select
    exists (
      select 1
      from auth.users
      where email = lower(trim(account_email))
    ) as account_exists,
    exists (
      select 1
      from auth.users u
      left join public.profiles p on p.id = u.id
      where u.email = lower(trim(account_email))
        and (
          p.password_set_at is not null
          or u.raw_user_meta_data ? 'password_set_at'
        )
    ) as password_set;
$$;

revoke all on function public.get_auth_account_status(text) from public;
grant execute on function public.get_auth_account_status(text) to anon, authenticated;

insert into public.profiles (id, email, password_set_at)
select
  id,
  lower(email),
  nullif(raw_user_meta_data->>'password_set_at', '')::timestamptz
from auth.users
where email is not null
on conflict (id) do update
set
  email = excluded.email,
  password_set_at = coalesce(public.profiles.password_set_at, excluded.password_set_at),
  updated_at = now();
